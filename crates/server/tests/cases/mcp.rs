use super::*;
use serde_json::{json, Value};

async fn manage(app: &App, method: &str, path: &str, token: &str, body: Value) -> (StatusCode, Value) {
    let response = router(app.clone()).oneshot(req(method, path, token, true, body.to_string())).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn discovery_and_calls_recheck_current_permissions_and_revoke() {
    let (_d, app, _, _) = setup().await;
    let (_, created) = manage(&app,"POST","/api/agent-connections","alice-token",json!({"name":"Agent"})).await;
    let token=created["token"].as_str().unwrap();
    let path=format!("/api/agent-connections/{}",created["connection"]["id"].as_str().unwrap());
    for send in [false,true] { for delete in [false,true] {
        assert_eq!(manage(&app,"PUT",&path,"alice-token",json!({"name":"Agent","allow_send":send,"allow_delete":delete})).await.0,StatusCode::OK);
        let (status, listed)=rpc(&app,token,json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).await;
        assert_eq!(status,StatusCode::OK);
        let names:Vec<_>=listed["result"]["tools"].as_array().unwrap().iter().map(|v|v["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"list_accounts"));
        assert_eq!(names.contains(&"send_draft"),send);
        for tool in ["trash_message","delete_message","empty_trash","delete_folder","delete_draft"] {
            assert_eq!(names.contains(&tool),delete,"{tool}");
            if !delete {
                let (_, denied)=rpc(&app,token,json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":tool,"arguments":{}}})).await;
                assert_eq!(denied["result"]["isError"],true);
                assert_eq!(denied["result"]["structuredContent"]["error"],"permission_denied");
            }
        }
        if !send {
            let (_,denied)=rpc(&app,token,json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"send_draft","arguments":{}}})).await;
            assert_eq!(denied["result"]["structuredContent"]["error"],"permission_denied");
        }
        for tool in ["request","batch","move_message","operation"] {
            let (_,denied)=rpc(&app,token,json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":tool,"arguments":{"path":"/api/accounts","change":{"kind":"trash"}}}})).await;
            assert_eq!(denied["result"]["structuredContent"]["error"],"unknown_tool");
        }
    }}
    manage(&app,"DELETE",&path,"alice-token",Value::Null).await;
    assert_eq!(rpc(&app,token,json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).await.0,StatusCode::UNAUTHORIZED);
}
async fn rpc(app: &App, token: &str, body: Value) -> (StatusCode,Value) {
    let response=router(app.clone()).oneshot(Request::builder().method("POST").uri("/mcp").header("authorization",format!("Bearer {token}")).header("content-type","application/json").header("accept","application/json, text/event-stream").body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status=response.status();
    let bytes=to_bytes(response.into_body(),2_000_000).await.unwrap();
    (status,serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn all_drafts_new_accounts_isolation_and_durable_concurrent_send_ids() {
    let (dir, app, alice, bob)=setup().await;
    let (_,created)=manage(&app,"POST","/api/agent-connections","alice-token",json!({"name":"Assistant"})).await;
    let token=created["token"].as_str().unwrap();
    let path=format!("/api/agent-connections/{}",created["connection"]["id"].as_str().unwrap());
    manage(&app,"PUT",&path,"alice-token",json!({"name":"Assistant","allow_send":true,"allow_delete":false})).await;
    let input:ConnectAccount=serde_json::from_value(json!({"email":"synthetic@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"fixture","password":"synthetic-only"})).unwrap();
    let first=app.store.connect(&alice,&input).await.unwrap();
    let foreign=app.store.connect(&bob,&input).await.unwrap();
    let a=app.store.authorize(&alice,&first.id).await.unwrap();
    let d:Draft=serde_json::from_value(json!({"id":"human-draft","account_id":first.id,"revision":0,"to":[{"email":"recipient@example.invalid","name":null}],"cc":[],"bcc":[],"subject":"Synthetic confidential subject","text":"Synthetic private body must never enter audit","html":"","in_reply_to":null,"references":[],"attachments":[],"saved_at":""})).unwrap();
    let d=app.store.save_draft(&a,d).await.unwrap();
    let listed=call(&app,token,"list_drafts",json!({"account_id":first.id})).await;
    assert_eq!(listed["rows"][0]["id"],d.id);
    assert_eq!(call(&app,token,"read_draft",json!({"account_id":foreign.id,"draft_id":d.id})).await["error"],"not_found");
    let second=app.store.connect(&alice,&input).await.unwrap();
    let accounts=call(&app,token,"list_accounts",json!({})).await;
    assert_eq!(accounts["rows"].as_array().unwrap().len(),2);
    let args=json!({"account_id":first.id,"draft_id":d.id,"revision":d.revision,"request_id":"send-unique"});
    let (one,two)=tokio::join!(call(&app,token,"send_draft",args.clone()),call(&app,token,"send_draft",args.clone()));
    assert!(one["id"].is_string(),"{one}");
    assert_eq!(one["id"],two["id"]);
    let changed=call(&app,token,"send_draft",json!({"account_id":second.id,"draft_id":d.id,"revision":d.revision,"request_id":"send-unique"})).await;
    assert_eq!(changed["error"],"revision_conflict");
    let mut restarted=app.clone();
    restarted.store=Store::open(dir.path(),[7;32]).await.unwrap();
    let retried=call(&restarted,token,"send_draft",args).await;
    assert_eq!(retried["id"],one["id"]);
    assert_eq!(call(&restarted,token,"get_send_status",json!({"request_id":"send-unique"})).await["id"],one["id"]);
    assert_eq!(app.store.submissions(&a).await.unwrap().len(),1);
    manage(&app,"PUT",&path,"alice-token",json!({"name":"Assistant","allow_send":false,"allow_delete":true})).await;
    let deleted=call(&app,token,"delete_draft",json!({"account_id":first.id,"draft_id":d.id,"revision":d.revision})).await;
    assert_eq!(deleted["deleted"],true,"{deleted}");
    assert!(app.store.draft(&a,&d.id).await.is_err());
}
async fn call(app:&App,token:&str,tool:&str,args:Value)->Value {
    let (status,response)=rpc(app,token,json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":tool,"arguments":args}})).await;
    assert_eq!(status,StatusCode::OK);
    response["result"]["structuredContent"].clone()
}

#[tokio::test]
async fn oauth_pkce_resource_binding_one_time_code_and_live_connection_permissions() {
    let (_d,app,_,_)=setup().await;
    let (_,created)=manage(&app,"POST","/api/agent-connections","alice-token",json!({"name":"ChatGPT"})).await;
    let client=public_json(&app,"/oauth/register",json!({"client_name":"Fixture client","redirect_uris":["https://client.example.invalid/callback"],"token_endpoint_auth_method":"none","grant_types":["authorization_code","refresh_token"],"response_types":["code"]})).await;
    assert_eq!(client.0,StatusCode::CREATED);
    let client_id=client.1["client_id"].as_str().unwrap();
    let verifier="abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG";
    let challenge=URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let parameters=json!({"client_id":client_id,"redirect_uri":"https://client.example.invalid/callback","response_type":"code","code_challenge":challenge,"code_challenge_method":"S256","resource":"https://dakia.example.invalid/mcp","scope":"mail","state":"fixture-state"});
    let mut pairs=url::form_urlencoded::Serializer::new(String::new());
    for (k,v) in parameters.as_object().unwrap() {pairs.append_pair(k,v.as_str().unwrap());}
    let query=pairs.finish();
    let page=router(app.clone()).oneshot(req("GET",&format!("/oauth/authorize?{query}"),"alice-token",false,String::new())).await.unwrap();
    assert_eq!(page.status(),StatusCode::OK);
    let approval=format!("{query}&csrf=csrf-value&connection_id={}&decision=approve",created["connection"]["id"].as_str().unwrap());
    let response=router(app.clone()).oneshot(Request::builder().method("POST").uri("/oauth/authorize").header("cookie","__Host-dakia=alice-token").header("origin","https://dakia.example.invalid").header("content-type","application/x-www-form-urlencoded").body(Body::from(approval)).unwrap()).await.unwrap();
    assert_eq!(response.status(),StatusCode::SEE_OTHER);
    let location=url::Url::parse(response.headers()["location"].to_str().unwrap()).unwrap();
    let code=location.query_pairs().find(|(k,_)|k=="code").unwrap().1.into_owned();
    let form=format!("grant_type=authorization_code&code={code}&client_id={client_id}&redirect_uri=https%3A%2F%2Fclient.example.invalid%2Fcallback&resource=https%3A%2F%2Fdakia.example.invalid%2Fmcp&code_verifier={verifier}");
    assert_eq!(public_form(&app,"/oauth/token",&form.replace(verifier,"bad-verifier")).await.0,StatusCode::BAD_REQUEST);
    let (status,tokens)=public_form(&app,"/oauth/token",&form).await;
    assert_eq!(status,StatusCode::OK,"{tokens}");
    assert_eq!(public_form(&app,"/oauth/token",&form).await.0,StatusCode::BAD_REQUEST);
    let access=tokens["access_token"].as_str().unwrap();
    assert_eq!(rpc(&app,access,json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).await.0,StatusCode::OK);
    let path=format!("/api/agent-connections/{}",created["connection"]["id"].as_str().unwrap());
    manage(&app,"DELETE",&path,"alice-token",Value::Null).await;
    assert_eq!(rpc(&app,access,json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).await.0,StatusCode::UNAUTHORIZED);
    let refresh=format!("grant_type=refresh_token&refresh_token={}&client_id={client_id}&resource=https%3A%2F%2Fdakia.example.invalid%2Fmcp",tokens["refresh_token"].as_str().unwrap());
    assert_eq!(public_form(&app,"/oauth/token",&refresh).await.0,StatusCode::BAD_REQUEST);
}
async fn public_json(app:&App,path:&str,value:Value)->(StatusCode,Value) {
    let r=router(app.clone()).oneshot(Request::builder().method("POST").uri(path).header("content-type","application/json").body(Body::from(value.to_string())).unwrap()).await.unwrap();
    let status=r.status();let bytes=to_bytes(r.into_body(),100000).await.unwrap();(status,serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}
async fn public_form(app:&App,path:&str,value:&str)->(StatusCode,Value) {
    let r=router(app.clone()).oneshot(Request::builder().method("POST").uri(path).header("content-type","application/x-www-form-urlencoded").body(Body::from(value.to_owned())).unwrap()).await.unwrap();
    let status=r.status();let bytes=to_bytes(r.into_body(),100000).await.unwrap();(status,serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn connections_are_private_default_safe_and_token_is_shown_once() {
    let (_d, app, _, _) = setup().await;
    let (status, created) = manage(&app, "POST", "/api/agent-connections", "alice-token", json!({"name":"Research"})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(created["connection"]["allow_send"], false);
    assert_eq!(created["connection"]["allow_delete"], false);
    let token = created["token"].as_str().unwrap();
    assert!(token.len() >= 43);
    let (status, listed) = manage(&app, "GET", "/api/agent-connections", "alice-token", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert!(!listed.to_string().contains(token));
    let (_, others) = manage(&app, "GET", "/api/agent-connections", "bob-token", Value::Null).await;
    assert_eq!(others, json!([]));
    let path = format!("/api/agent-connections/{}", created["connection"]["id"].as_str().unwrap());
    assert_eq!(manage(&app, "PUT", &path, "bob-token", json!({"name":"stolen","allow_send":true,"allow_delete":true})).await.0, StatusCode::NOT_FOUND);
    assert_eq!(manage(&app, "PUT", &path, "alice-token", json!({"name":"Assistant","allow_send":true,"allow_delete":false})).await.0, StatusCode::OK);
    assert_eq!(manage(&app, "DELETE", &path, "alice-token", Value::Null).await.0, StatusCode::NO_CONTENT);
}
