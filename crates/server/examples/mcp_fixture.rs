//! Seed only an explicit new disposable directory. Never opens an existing instance.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD,Engine};
use dakia_domain::*;
use dakia_storage::Store;
use serde_json::json;
use sha2::{Digest,Sha256};
#[tokio::main]
async fn main()->Result<(),Box<dyn std::error::Error>> {
 let mut args=std::env::args().skip(1);let root=std::path::PathBuf::from(args.next().ok_or("directory required")?);
 if root.exists(){return Err("fixture directory must not exist".into())}
 let imap:u16=args.next().ok_or("IMAP port required")?.parse()?;let smtp:u16=args.next().ok_or("SMTP port required")?.parse()?;
 std::fs::create_dir_all(&root)?;std::fs::write(root.join("key"),[9;32])?;std::fs::write(root.join("oidc"),"fixture-only")?;
 let store=Store::open(root.join("data"),[9;32]).await?;
 let mut accounts=Vec::new();
 for (subject,token) in [("alice","alice-fixture"),("bob","bob-fixture")] {
  let user=store.user("https://fixture.invalid",subject).await?;
  store.control_repository().create_session(&URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes())),&user,"fixture-csrf",4102444800).await?;
  let input:ConnectAccount=serde_json::from_value(json!({"email":format!("{subject}@example.invalid"),"display_name":format!("{subject} synthetic"),"provider":"imap","imap_host":"localhost","imap_port":imap,"imap_tls":"implicit","smtp_host":"localhost","smtp_port":smtp,"smtp_tls":"implicit","username":subject,"password":"fixture-only-password"}))?;
  let account=store.connect(&user,&input).await?;let a=store.authorize(&user,&account.id).await?;
  store.account_state(&a,AccountState::Ready).await?;
  let d:Draft=serde_json::from_value(json!({"id":format!("{subject}-human-draft"),"account_id":a.id(),"revision":0,"to":[{"email":"sink@example.invalid","name":null}],"cc":[],"bcc":[],"subject":"Synthetic MCP send","text":"Synthetic private body audit sentinel","html":"","in_reply_to":null,"references":[],"attachments":[],"saved_at":""}))?;
  let d=store.save_draft(&a,d).await?;
  accounts.push(json!({"user":subject,"account_id":a.id(),"draft_id":d.id,"revision":d.revision}));
 }
 println!("{}",json!({"accounts":accounts}));Ok(())
}
