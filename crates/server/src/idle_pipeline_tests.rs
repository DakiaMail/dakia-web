use super::*;

#[tokio::test]
#[ignore = "explicit disposable IDLE-to-Web-Push pipeline; requires installed Node push worker"]
async fn disposable_idle_to_push_pipeline_replays_without_fresh_notification_identity() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let worker = root.join("workers/push");
    let keys = Command::new("node").args(["-e", "const c=require('node:crypto');let e=c.createECDH('prime256v1');e.generateKeys();process.stdout.write(JSON.stringify({p256dh:e.getPublicKey().toString('base64url'),auth:c.randomBytes(16).toString('base64url')}))"]).output().unwrap();
    assert!(keys.status.success());
    let keys: serde_json::Value = serde_json::from_slice(&keys.stdout).unwrap();
    let (_dir, app, user, a, provider) = paged(false).await;
    provider.heads.store(true, Ordering::SeqCst);
    provider.idle_hints.store(true, Ordering::SeqCst);
    let subscription = serde_json::json!({"id":"disposable","user_id":user,"endpoint":"https://fcm.googleapis.com/disposable","keys":keys});
    app.store
        .subscribe(&user, "disposable", &subscription.to_string())
        .await
        .unwrap();
    let listener = tokio::spawn(crate::idle::run(app.clone()));
    tokio::time::timeout(Duration::from_secs(10), async {
        while app
            .store
            .provider_document(&a, "sync:head:INBOX")
            .await
            .unwrap()
            .is_none()
        {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    assert!(app.store.next_push(&a).await.unwrap().is_none());
    let started = std::time::Instant::now();
    provider.arrival_uid.store(1, Ordering::SeqCst);
    let job = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            // The push queue now waits for a completed classifier decision.
            // This provider fixture returns already-seen mail, so supply the
            // synthetic completed decision directly (auto-classify is unread-only).
            for mut message in app.store.inbox_rows(&a, None, None).await.unwrap() {
                if message.category_status == dakia_domain::CategoryStatus::Pending {
                    message.category = Some("people".into());
                    message.category_status = dakia_domain::CategoryStatus::Ready;
                    app.store.upsert_message(&a, message).await.unwrap();
                }
            }
            if let Some(job) = app.store.next_push(&a).await.unwrap() {
                break job;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    app.store.recover_push(&a).await.unwrap();
    let replay = app.store.next_push(&a).await.unwrap().unwrap();
    assert_eq!(job, replay);
    let script = r#"
        import fs from 'node:fs'; import assert from 'node:assert/strict';
        import {runInNewContext} from 'node:vm'; import webpush from 'web-push';
        import {PushService} from './service.mjs';
        const job=JSON.parse(fs.readFileSync(0,'utf8'));
        const handlers=new Map(), visible=new Map();
        const self={location:{origin:'https://dakia.example.test'},addEventListener:(n,f)=>handlers.set(n,f),registration:{showNotification:async(t,o)=>{assert.equal(t,'New mail');assert.equal(o.renotify,false);visible.set(o.tag,o)}}};
        runInNewContext(fs.readFileSync('../../apps/web/public/sw.js','utf8'),{self,URL,AbortController,setTimeout,clearTimeout,fetch:async()=>{throw Error('synthetic offline')}});
        const vapid={subject:'mailto:disposable@example.test',...webpush.generateVAPIDKeys()};
        let sends=0;
        const service=new PushService(vapid,async(s,p,o)=>{
            sends++;const encrypted=webpush.generateRequestDetails(s,p,o);
            assert.equal(encrypted.headers['Content-Encoding'],'aes128gcm');
            assert.ok(!encrypted.body.includes(Buffer.from('New mail')));
            let pending;handlers.get('push')({data:{json:()=>JSON.parse(p)},waitUntil:p=>pending=p});await pending;
            return {statusCode:201};
        });
        assert.equal((await service.send(job)).status,'sent');
        assert.equal((await service.send(job)).status,'sent');
        assert.equal(sends,2);assert.equal(visible.size,1);
        console.log(JSON.stringify({synthetic_attempts:sends,visible_notification_identities:visible.size,encrypted:true,external_requests:0}));
    "#;
    let mut child = Command::new("node")
        .current_dir(worker)
        .args(["--input-type=module", "-e", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&job).unwrap())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    println!(
        "synthetic_hint_to_durable_push_ms={} {}",
        started.elapsed().as_millis(),
        String::from_utf8_lossy(&result.stdout)
    );
    app.store
        .finish_push(&a, job["message_id"].as_str().unwrap(), "disposable", None)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert!(app.store.next_push(&a).await.unwrap().is_none());
    listener.abort();
    let _ = listener.await;
}
