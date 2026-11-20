use super::*;
use dakia_mail_provider as p;
use dakia_storage::AuthorizedAccount;

async fn reference(app:&App,a:&AuthorizedAccount,id:&str)->Result<(p::MessageRef,String)> {
    app.store.message(a,id).await?;
    let raw=app.store.provider_document(a,&format!("reference:{id}")).await?.ok_or(ApiFailure(StatusCode::NOT_FOUND,"message_reference_unavailable"))?;
    let r:p::MessageRef=serde_json::from_str(&raw).map_err(|_|ApiFailure(StatusCode::CONFLICT,"message_reference_unavailable"))?;
    if r.account_id!=a.id() {return Err(ApiFailure(StatusCode::NOT_FOUND,"not_found"))}
    Ok((r,raw))
}
async fn allowed(app:&App,c:&AgentConnection)->Result<()> {
    let current=app.store.control_repository().agent(&c.user_id,&c.id).await?;
    if current.revoked||!current.allow_delete {return Err(ApiFailure(StatusCode::FORBIDDEN,"permission_denied"))}
    Ok(())
}
pub(super) async fn execute(app:&App,c:&AgentConnection,a:&AuthorizedAccount,tool:&str,args:&Value)->Result<Value> {
    let request=hash(field(args,"request_id")?);
    let payload=hash(&format!("{tool}:{}",args));
    let repo=app.store.control_repository();
    if let Some(result)=repo.claim_agent_deletion(&c.id,&request,&payload).await? {return serde_json::from_str(&result).map_err(|_|ApiFailure(StatusCode::INTERNAL_SERVER_ERROR,"stored_outcome_unavailable"))}
    // Persist the claim before any provider write. Interruption means unknown,
    // never permission to retry the destructive request automatically.
    let result=perform(app,c,a,tool,args).await;
    let document=match result {Ok(v)=>v,Err(e)=>json!({"error":e.1,"outcome":"not_completed"})};
    repo.finish_agent_deletion(&c.id,&request,&document.to_string()).await?;
    Ok(document)
}
async fn perform(app:&App,c:&AgentConnection,a:&AuthorizedAccount,tool:&str,args:&Value)->Result<Value> {
    let lane=app.mail.reconciliation_lane(a).await;
    let _guard=lane.lock().await;
    allowed(app,c).await?;
    let provider=app.mail.provider(&app.store,a).await?;
    if tool=="delete_folder" {
        let folder=field(args,"folder")?;
        let outcome=provider.delete_folder(folder).await;
        return Ok(match outcome {Ok(())=>json!({"deleted":true,"outcome":"confirmed"}),Err(e)=>json!({"deleted":false,"outcome":if e.kind==p::ErrorKind::Uncertain{"unknown"}else{"failed"},"error":if e.kind==p::ErrorKind::Unsupported{"provider_unsupported"}else{"deletion_not_confirmed"}})});
    }
    let ids:Vec<&str>=if tool=="empty_trash" {args["message_ids"].as_array().unwrap().iter().map(|v|v.as_str().unwrap()).collect()}else{vec![field(args,"message_id")?]};
    let trash=if tool=="empty_trash" {
        let settings=app.mail.settings(&app.store,a).await?;
        match settings.trash_folder {Some(name)=>Some(name),None=>{
            let folders=provider.folders().await.map_err(|_|ApiFailure(StatusCode::BAD_GATEWAY,"folder_discovery_failed"))?;
            let candidates:Vec<_>=folders.iter().filter(|f|f.attributes.iter().any(|a|a.eq_ignore_ascii_case("\\Trash"))).collect();
            if candidates.len()!=1 {return Err(ApiFailure(StatusCode::CONFLICT,"trash_folder_unavailable"))}Some(candidates[0].name.clone())
        }}
    }else{None};
    // Resolve and validate every explicit target before the first destructive effect.
    let mut targets=Vec::new();
    for id in ids {
        let (r,raw)=reference(app,a,id).await?;
        if trash.as_ref().is_some_and(|t|!r.folder.eq_ignore_ascii_case(t)) {return Err(ApiFailure(StatusCode::BAD_REQUEST,"target_not_in_trash"))}
        targets.push((id,r,raw));
    }
    let mut outcomes=Vec::new();
    for (id,r,raw) in targets {
        allowed(app,c).await?;
        match provider.delete_message(&r).await {
            Ok(())=>{
                let cache=app.store.invalidate_stale_reference(a,id,&r.folder,&raw).await;
                outcomes.push(json!({"message_id":id,"outcome":"confirmed","cache_reconciled":cache.is_ok()}));
            },
            Err(e)=>outcomes.push(json!({"message_id":id,"outcome":if e.kind==p::ErrorKind::Uncertain{"unknown"}else{"failed"},"error":if e.kind==p::ErrorKind::Unsupported{"provider_unsupported"}else{"deletion_not_confirmed"}})),
        }
    }
    Ok(json!({"results":outcomes}))
}
