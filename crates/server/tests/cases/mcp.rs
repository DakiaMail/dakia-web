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
