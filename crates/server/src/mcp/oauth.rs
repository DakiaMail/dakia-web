//! Public-client OAuth authorization-code flow, S256 PKCE and resource-bound rotating grants.
use super::*;
use axum::{extract::Form,response::Html};
use dakia_storage::control_repository::{AgentOAuthCode,AgentOAuthToken};

pub(crate) async fn resource(State(app):State<App>)->Json<Value> { Json(json!({"resource":format!("{}/mcp",app.config.origin),"authorization_servers":[app.config.origin],"scopes_supported":["mail"],"bearer_methods_supported":["header"],"resource_name":"Dakia Mail"})) }
pub(crate) async fn metadata(State(app):State<App>)->Json<Value> {
    let o=&app.config.origin;
    Json(json!({"issuer":o,"authorization_endpoint":format!("{o}/oauth/authorize"),"token_endpoint":format!("{o}/oauth/token"),"registration_endpoint":format!("{o}/oauth/register"),"response_types_supported":["code"],"grant_types_supported":["authorization_code","refresh_token"],"token_endpoint_auth_methods_supported":["none"],"code_challenge_methods_supported":["S256"],"scopes_supported":["mail"],"authorization_response_iss_parameter_supported":true}))
}
#[derive(serde::Serialize,Deserialize)]
pub(crate) struct Registration {
    client_name:Option<String>, redirect_uris:Vec<String>,token_endpoint_auth_method:Option<String>,grant_types:Option<Vec<String>>,response_types:Option<Vec<String>>,
}
fn invalid()->ApiFailure {ApiFailure(StatusCode::BAD_REQUEST,"invalid_grant")}
fn redirect_allowed(s:&str)->bool {
    url::Url::parse(s).ok().is_some_and(|u| !u.has_host()==false && u.username().is_empty()&&u.password().is_none()&&u.fragment().is_none() && (u.scheme()=="https" || (u.scheme()=="http" && matches!(u.host_str(),Some("127.0.0.1"|"[::1]"|"localhost")))))
}
pub(crate) async fn register(State(app):State<App>,Json(input):Json<Registration>)->Result<Response> {
    let attempts=app.store.control_repository().record_login_attempt("mcp:registration",Utc::now().timestamp()).await?;
    if attempts>20 {return Err(ApiFailure(StatusCode::TOO_MANY_REQUESTS,"try_again_later"))}
    if input.redirect_uris.is_empty()||input.redirect_uris.len()>10||input.redirect_uris.iter().any(|s|s.len()>2048||!redirect_allowed(s))||input.client_name.as_ref().is_some_and(|s|s.len()>100)||input.token_endpoint_auth_method.as_deref().is_some_and(|s|s!="none")||input.response_types.as_ref().is_some_and(|v|v!=&vec!["code".to_string()])||input.grant_types.as_ref().is_some_and(|v|v.iter().any(|s|s!="authorization_code"&&s!="refresh_token")) {return Err(ApiFailure(StatusCode::BAD_REQUEST,"invalid_client_metadata"))}
    let id=random();
    app.store.control_repository().register_agent_client(&id,&serde_json::to_string(&input).map_err(|_|invalid())?,Utc::now().timestamp()+365*86400).await?;
    Ok((StatusCode::CREATED,Json(json!({"client_id":id,"client_name":input.client_name,"redirect_uris":input.redirect_uris,"token_endpoint_auth_method":"none","grant_types":["authorization_code","refresh_token"],"response_types":["code"]}))).into_response())
}
#[derive(Deserialize,serde::Serialize)]
pub(crate) struct Authorization {
    client_id:String,redirect_uri:String,response_type:String,code_challenge:String,code_challenge_method:String,resource:String,
    #[serde(default)] scope:String,
    #[serde(default)] state:String,
}
async fn validate(app:&App,a:&Authorization)->Result<Registration> {
    if a.response_type!="code"||a.code_challenge_method!="S256"||a.code_challenge.len()!=43||URL_SAFE_NO_PAD.decode(&a.code_challenge).ok().is_none_or(|v|v.len()!=32)||a.resource!=format!("{}/mcp",app.config.origin)||!matches!(a.scope.as_str(),""|"mail")||a.state.len()>2048 {return Err(ApiFailure(StatusCode::BAD_REQUEST,"invalid_authorization_request"))}
    let raw=app.store.control_repository().agent_client(&a.client_id,Utc::now().timestamp()).await?.ok_or(ApiFailure(StatusCode::BAD_REQUEST,"invalid_client"))?;
    let client:Registration=serde_json::from_str(&raw).map_err(|_|invalid())?;
    if !client.redirect_uris.contains(&a.redirect_uri) {return Err(ApiFailure(StatusCode::BAD_REQUEST,"invalid_redirect_uri"))}
    Ok(client)
}
fn escape(s:&str)->String {s.replace('&',"&amp;").replace('<',"&lt;").replace('>',"&gt;").replace('"',"&quot;").replace('\'',"&#39;")}
async fn browser_identity(app:&App,headers:&HeaderMap)->Result<Identity> {
    let token=cookie(headers,"__Host-dakia").ok_or(ApiFailure(StatusCode::UNAUTHORIZED,"login_required"))?;
    identity(app,&hash(&token)).await
}
pub(crate) async fn authorize(State(app):State<App>,headers:HeaderMap,Query(a):Query<Authorization>)->Result<Response> {
    let client=validate(&app,&a).await?;
    let identity=browser_identity(&app,&headers).await;
    let (content,title)=match identity {
        Err(_)=>("<p>Sign in to Dakia, then return to this tab to continue.</p><p><a href=\"/auth/login\" target=\"_blank\" rel=\"noopener\">Sign in to Dakia</a></p><button onclick=\"location.reload()\">Continue</button>".to_owned(),"Connect your mail"),
        Ok(id)=>{
            let connections=app.store.control_repository().agents(&id.user).await?;
            let fields=serde_json::to_value(&a).map_err(|_|invalid())?.as_object().unwrap().iter().map(|(k,v)|format!("<input type=\"hidden\" name=\"{}\" value=\"{}\">",escape(k),escape(v.as_str().unwrap_or("")))).collect::<String>();
            let options=connections.iter().map(|c|format!("<option value=\"{}\">{} · Sending {} · Deleting {}</option>",escape(&c.id),escape(&c.name),if c.allow_send{"on"}else{"off"},if c.allow_delete{"on"}else{"off"})).collect::<String>();
            let body=if connections.is_empty() {"<p>Create an agent connection in Workspace first, then return here.</p><a href=\"/\" target=\"_blank\" rel=\"noopener\">Open Dakia</a>".to_owned()} else {format!("<p><strong>{}</strong> wants to use your mail through Dakia.</p><p>The connection includes all your email accounts and drafts. Its sending and deletion settings apply to this client.</p><form method=\"post\" action=\"/oauth/authorize\">{fields}<input type=\"hidden\" name=\"csrf\" value=\"{}\"><label for=\"connection\">Agent connection</label><select id=\"connection\" name=\"connection_id\" required autofocus>{options}</select><div class=\"actions\"><button name=\"decision\" value=\"approve\">Connect</button><button class=\"secondary\" name=\"decision\" value=\"deny\">Cancel</button></div></form>",escape(client.client_name.as_deref().unwrap_or("MCP client")),escape(&id.csrf))};
            (body,"Connect to Dakia")
        }
    };
    Ok(([(header::CONTENT_SECURITY_POLICY,"default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'"),(header::X_FRAME_OPTIONS,"DENY")],Html(format!("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{title}</title><style>:root{{color-scheme:light dark}}*{{box-sizing:border-box}}body{{font:16px system-ui;margin:0;padding:24px;min-height:100dvh;display:grid;place-items:center;background:light-dark(#f5f5f7,#141416);color:light-dark(#202124,#ececef)}}main{{width:100%;max-width:480px;background:light-dark(white,#242428);border:1px solid light-dark(#e4e4e7,#3f3f46);padding:32px;border-radius:20px}}h1{{font-size:26px;margin-top:0}}p{{line-height:1.6}}label{{display:block;margin-top:24px}}select{{font:inherit;width:100%;padding:12px;margin:8px 0 24px;min-height:44px}}.actions{{display:flex;gap:12px;flex-wrap:wrap}}button,a{{font:inherit;min-height:44px;padding:12px 18px;border-radius:8px}}button{{background:light-dark(#27272a,#e4e4e7);color:light-dark(white,#18181b);border:1px solid transparent;cursor:pointer}}.secondary{{background:transparent;color:inherit;border-color:light-dark(#ccc,#666)}}:focus-visible{{outline:3px solid #5789ff;outline-offset:3px}}@media(max-width:420px){{main{{padding:24px}}body{{padding:16px}}}}</style><main><h1>{title}</h1>{content}</main></html>"))).into_response())
}
#[derive(Deserialize)]
pub(crate) struct Approval {
    #[serde(flatten)] authorization:Authorization,
    csrf:String,connection_id:String,decision:String,
}
pub(crate) async fn approve(State(app):State<App>,headers:HeaderMap,Form(input):Form<Approval>)->Result<Response> {
    let id=browser_identity(&app,&headers).await?;
    if input.csrf!=id.csrf||headers.get(header::ORIGIN).and_then(|v|v.to_str().ok())!=Some(&app.config.origin) {return Err(ApiFailure(StatusCode::FORBIDDEN,"csrf_rejected"))}
    validate(&app,&input.authorization).await?;
    let c=app.store.control_repository().agent(&id.user,&input.connection_id).await?;
    if c.revoked {return Err(ApiFailure(StatusCode::BAD_REQUEST,"connection_revoked"))}
    let a=input.authorization;
    let mut redirect=url::Url::parse(&a.redirect_uri).map_err(|_|invalid())?;
    if input.decision=="approve" {
        let code=random();
        app.store.control_repository().create_agent_code(&AgentOAuthCode{hash:hash(&code),connection:c.id.clone(),client:a.client_id,redirect:a.redirect_uri.clone(),challenge:a.code_challenge,resource:a.resource,expires:Utc::now().timestamp()+120}).await?;
        app.store.control_repository().audit_agent(&c,"oauth_approve","ok",None,None,None).await?;
        redirect.query_pairs_mut().append_pair("code",&code);
    } else if input.decision=="deny" {redirect.query_pairs_mut().append_pair("error","access_denied");} else {return Err(invalid())}
    redirect.query_pairs_mut().append_pair("state",&a.state).append_pair("iss",&app.config.origin);
    Ok(Redirect::to(redirect.as_str()).into_response())
}
#[derive(Deserialize)]
pub(crate) struct TokenRequest {
    grant_type:String,client_id:String,resource:String,
    code:Option<String>,code_verifier:Option<String>,redirect_uri:Option<String>,refresh_token:Option<String>,
}
pub(crate) async fn token(State(app):State<App>,Form(input):Form<TokenRequest>)->Response {
    match exchange(&app,input).await {Ok(v)=>Json(v).into_response(),Err(e) if e.0==StatusCode::BAD_REQUEST=>(StatusCode::BAD_REQUEST,Json(json!({"error":"invalid_grant"}))).into_response(),Err(e)=>e.into_response()}
}
async fn exchange(app:&App,input:TokenRequest)->Result<Value> {
    let at=Utc::now().timestamp();
    if input.resource!=format!("{}/mcp",app.config.origin)||input.client_id.len()>256 {return Err(invalid())}
    let access=random();let refresh=random();
    let mut t=AgentOAuthToken{access_hash:hash(&access),refresh_hash:hash(&refresh),connection:String::new(),client:input.client_id,resource:input.resource,expires:at+3600,refresh_expires:at+30*86400};
    if input.grant_type=="authorization_code" {
        let verifier=input.code_verifier.ok_or_else(invalid)?;
        if !(43..=128).contains(&verifier.len())||!verifier.bytes().all(|c|c.is_ascii_alphanumeric()||b"-._~".contains(&c)) {return Err(invalid())}
        let c=AgentOAuthCode{hash:hash(&input.code.ok_or_else(invalid)?),connection:String::new(),client:t.client.clone(),redirect:input.redirect_uri.ok_or_else(invalid)?,challenge:hash(&verifier),resource:t.resource.clone(),expires:at};
        t.connection=app.store.control_repository().consume_agent_code(&c,at).await?.ok_or_else(invalid)?;
        app.store.control_repository().create_agent_token(&t).await?;
    } else if input.grant_type=="refresh_token" {
        if !app.store.control_repository().rotate_agent_token(&hash(&input.refresh_token.ok_or_else(invalid)?),&t,at).await? {return Err(invalid())}
    } else {return Err(invalid())}
    Ok(json!({"access_token":access,"token_type":"Bearer","expires_in":3600,"refresh_token":refresh,"scope":"mail"}))
}
