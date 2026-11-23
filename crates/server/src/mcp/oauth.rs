//! Public-client OAuth authorization-code flow, S256 PKCE and resource-bound rotating grants.
use super::*;
use axum::{
    extract::{ConnectInfo, Form, OriginalUri},
    response::Html,
    Extension,
};
use dakia_storage::control_repository::{AgentOAuthCode, AgentOAuthToken};
use std::net::{IpAddr, SocketAddr};

pub(crate) async fn resource(State(app): State<App>) -> Json<Value> {
    Json(
        json!({"resource":format!("{}/mcp",app.config.origin),"authorization_servers":[app.config.origin],"scopes_supported":["mail"],"bearer_methods_supported":["header"],"resource_name":"Dakia"}),
    )
}
pub(crate) async fn metadata(State(app): State<App>) -> Json<Value> {
    let o = &app.config.origin;
    Json(
        json!({"issuer":o,"authorization_endpoint":format!("{o}/oauth/authorize"),"token_endpoint":format!("{o}/oauth/token"),"registration_endpoint":format!("{o}/oauth/register"),"response_types_supported":["code"],"grant_types_supported":["authorization_code","refresh_token"],"token_endpoint_auth_methods_supported":["none"],"code_challenge_methods_supported":["S256"],"scopes_supported":["mail"],"authorization_response_iss_parameter_supported":true}),
    )
}
#[derive(serde::Serialize, Deserialize)]
pub(crate) struct Registration {
    client_name: Option<String>,
    redirect_uris: Vec<String>,
    token_endpoint_auth_method: Option<String>,
    grant_types: Option<Vec<String>>,
    response_types: Option<Vec<String>>,
}
fn invalid() -> ApiFailure {
    ApiFailure(StatusCode::BAD_REQUEST, "invalid_grant")
}
fn redirect_allowed(s: &str) -> bool {
    url::Url::parse(s).ok().is_some_and(|u| {
        u.has_host()
            && u.username().is_empty()
            && u.password().is_none()
            && u.fragment().is_none()
            && (u.scheme() == "https"
                || (u.scheme() == "http"
                    && matches!(u.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"))))
    })
}
pub(crate) async fn register(
    State(app): State<App>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(input): Json<Registration>,
) -> Result<Response> {
    if input.redirect_uris.is_empty()
        || input.redirect_uris.len() > 10
        || input
            .redirect_uris
            .iter()
            .any(|s| s.len() > 2048 || !redirect_allowed(s))
        || input.client_name.as_ref().is_some_and(|s| s.len() > 100)
        || input
            .token_endpoint_auth_method
            .as_deref()
            .is_some_and(|s| s != "none")
        || input
            .response_types
            .as_ref()
            .is_some_and(|v| v != &vec!["code".to_string()])
        || input.grant_types.as_ref().is_some_and(|v| {
            v.iter()
                .any(|s| s != "authorization_code" && s != "refresh_token")
        })
    {
        return Err(ApiFailure(
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
        ));
    }
    let forwarded_ip =
        if std::env::var("DAKIA_TRUST_REGISTRATION_CLIENT_IP").as_deref() == Ok("true") {
            headers
                .get("Dakia-Client-IP")
                .and_then(|value| value.to_str().ok()?.parse::<IpAddr>().ok())
        } else {
            None
        };
    let source = forwarded_ip.or_else(|| peer.map(|Extension(ConnectInfo(addr))| addr.ip()));
    let bucket = match source.map(|ip| ip.to_canonical()) {
        Some(IpAddr::V6(ip)) => format!("mcp:registration:v6:{:016x}", u128::from(ip) >> 64),
        Some(IpAddr::V4(ip)) => format!("mcp:registration:v4:{ip}"),
        None => "mcp:registration:unknown".into(),
    };
    let attempts = app
        .store
        .control_repository()
        .record_login_attempt(&bucket, Utc::now().timestamp())
        .await?;
    if attempts > 20 {
        return Err(ApiFailure(StatusCode::TOO_MANY_REQUESTS, "try_again_later"));
    }
    let id = random();
    app.store
        .control_repository()
        .register_agent_client(
            &id,
            &serde_json::to_string(&input).map_err(|_| invalid())?,
            Utc::now().timestamp() + 365 * 86400,
        )
        .await?;
    Ok((StatusCode::CREATED,Json(json!({"client_id":id,"client_name":input.client_name,"redirect_uris":input.redirect_uris,"token_endpoint_auth_method":"none","grant_types":["authorization_code","refresh_token"],"response_types":["code"]}))).into_response())
}
#[derive(Deserialize, serde::Serialize)]
pub(crate) struct Authorization {
    client_id: String,
    redirect_uri: String,
    response_type: String,
    code_challenge: String,
    code_challenge_method: String,
    resource: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    state: String,
}
async fn validate(app: &App, a: &Authorization) -> Result<Registration> {
    if a.response_type != "code"
        || a.code_challenge_method != "S256"
        || a.code_challenge.len() != 43
        || URL_SAFE_NO_PAD
            .decode(&a.code_challenge)
            .ok()
            .is_none_or(|v| v.len() != 32)
        || a.resource != format!("{}/mcp", app.config.origin)
        || !matches!(a.scope.as_str(), "" | "mail")
        || a.state.len() > 2048
    {
        return Err(ApiFailure(
            StatusCode::BAD_REQUEST,
            "invalid_authorization_request",
        ));
    }
    let raw = app
        .store
        .control_repository()
        .agent_client(&a.client_id, Utc::now().timestamp())
        .await?
        .ok_or(ApiFailure(StatusCode::BAD_REQUEST, "invalid_client"))?;
    let client: Registration = serde_json::from_str(&raw).map_err(|_| invalid())?;
    if !client.redirect_uris.contains(&a.redirect_uri) {
        return Err(ApiFailure(StatusCode::BAD_REQUEST, "invalid_redirect_uri"));
    }
    Ok(client)
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
async fn browser_identity(app: &App, headers: &HeaderMap) -> Result<Identity> {
    let token = cookie(headers, "__Host-dakia")
        .ok_or(ApiFailure(StatusCode::UNAUTHORIZED, "login_required"))?;
    identity(app, &hash(&token)).await
}
const CONSENT_TOKENS: &str = include_str!("../../../../packages/ui/src/tokens.css");
const CONSENT_STYLE: &str = include_str!("consent.css");

pub(crate) async fn authorize(
    State(app): State<App>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Query(a): Query<Authorization>,
) -> Result<Response> {
    let client = validate(&app, &a).await?;
    let callback = url::Url::parse(&a.redirect_uri).map_err(|_| invalid())?;
    let callback_origin = callback.origin().ascii_serialization();
    let name = escape(client_display_name(&client));
    let body = match browser_identity(&app, &headers).await {
        Ok(id) => {
            let connections = app.store.control_repository().agents(&id.user).await?;
            let reuse = if connections.is_empty() {
                String::new()
            } else {
                let options: String = connections
                    .iter()
                    .map(|c| {
                        let permissions = match (c.allow_send, c.allow_delete) {
                            (true, true) => " (can send and permanently delete)",
                            (true, false) => " (can send)",
                            (false, true) => " (can permanently delete)",
                            (false, false) => "",
                        };
                        format!(
                            "<option value=\"{}\">{}{permissions}</option>",
                            escape(&c.id),
                            escape(&c.name)
                        )
                    })
                    .collect();
                format!(
                    r#"<label for="connection">Agent connection</label><select id="connection" name="connection_id"><option value="">Create a new connection</option>{options}</select>"#
                )
            };
            let document = serde_json::to_value(&a).map_err(|_| invalid())?;
            let fields: String = document
                .as_object()
                .ok_or_else(invalid)?
                .iter()
                .filter_map(|(key, value)| {
                    let value = value.as_str()?;
                    Some(format!(
                        r#"<input type="hidden" name="{}" value="{}">"#,
                        escape(key),
                        escape(value)
                    ))
                })
                .collect();
            format!(
                r#"<p><strong>{name}</strong> is requesting access to all your mail accounts and drafts.</p><p class="callback">Returns to {}</p><form method="post" action="/oauth/authorize">{fields}<input type="hidden" name="csrf" value="{}">{reuse}<div class="actions"><button class="primary" name="decision" value="approve">Connect</button><button name="decision" value="deny">Cancel</button></div></form>"#,
                escape(&callback_origin),
                escape(&id.csrf),
            )
        }
        Err(ApiFailure(StatusCode::UNAUTHORIZED, _)) => {
            let next = uri.to_string();
            if login_target(&next).is_some() {
                let query = url::form_urlencoded::Serializer::new(String::new())
                    .append_pair("next", &next)
                    .finish();
                format!(
                    r#"<p>Sign in to connect <strong>{name}</strong> to your mail.</p><div class="actions"><a class="primary" href="/auth/login?{}">Sign in</a></div>"#,
                    escape(&query)
                )
            } else {
                format!(
                    r#"<p>Sign in to connect <strong>{name}</strong> to your mail.</p><div class="actions"><a class="primary" href="/auth/login" target="_blank" rel="noopener">Sign in</a><a href="{}">Continue</a></div>"#,
                    escape(&next)
                )
            }
        }
        Err(error) => return Err(error),
    };
    // Chromium checks the registered callback origin as well as the form's initial destination.
    let nonce = random();
    let csp = format!("default-src 'none'; img-src 'self'; style-src 'nonce-{nonce}'; script-src 'nonce-{nonce}'; form-action 'self' {callback_origin}; frame-ancestors 'none'; base-uri 'none'");
    Ok((
        [
            (header::CONTENT_SECURITY_POLICY, csp.as_str()),
            (header::X_FRAME_OPTIONS, "DENY"),
        ],
        Html(format!(
            r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Connect to Dakia</title><link rel="icon" href="/dakia-icon-192.png" type="image/png">
<script nonce="{nonce}">
let theme;
try {{ theme = localStorage.getItem("dakia-theme"); }} catch {{}}
document.documentElement.dataset.theme = theme === "dark"
    || (theme === "auto" && matchMedia("(prefers-color-scheme: dark)").matches) ? "dark" : "light";
</script><style nonce="{nonce}">{CONSENT_TOKENS}{CONSENT_STYLE}</style></head>
<body><main><img src="/dakia-icon-192.png" alt="" width="44" height="44"><h1>Connect to Dakia</h1>{body}</main></body></html>"#,
        )),
    )
        .into_response())
}

fn client_display_name(client: &Registration) -> &str {
    client
        .client_name
        .as_deref()
        .and_then(|name| super::name(name).ok())
        .unwrap_or("AI agent")
}

#[derive(Deserialize)]
pub(crate) struct Approval {
    #[serde(flatten)]
    authorization: Authorization,
    csrf: String,
    #[serde(default)]
    connection_id: String,
    decision: String,
}

pub(crate) async fn approve(
    State(app): State<App>,
    headers: HeaderMap,
    Form(input): Form<Approval>,
) -> Result<Response> {
    let id = browser_identity(&app, &headers).await?;
    if input.csrf != id.csrf
        || headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) != Some(&app.config.origin)
    {
        return Err(ApiFailure(StatusCode::FORBIDDEN, "csrf_rejected"));
    }
    let a = input.authorization;
    let client = validate(&app, &a).await?;
    let mut redirect = url::Url::parse(&a.redirect_uri).map_err(|_| invalid())?;
    if input.decision == "approve" {
        let repo = app.store.control_repository();
        let new_connection = input.connection_id.is_empty();
        let connection = if new_connection {
            uuid::Uuid::new_v4().to_string()
        } else {
            let c = repo.agent(&id.user, &input.connection_id).await?;
            if c.revoked {
                return Err(ApiFailure(StatusCode::BAD_REQUEST, "connection_revoked"));
            }
            c.id
        };
        let code = random();
        // OAuth-only connections have no issued bearer credential.
        let unused_token_hash = new_connection.then(|| hash(&random()));
        repo.create_agent_code(
            &AgentOAuthCode {
                hash: hash(&code),
                connection: connection.clone(),
                client: a.client_id,
                redirect: a.redirect_uri.clone(),
                challenge: a.code_challenge,
                resource: a.resource,
                expires: Utc::now().timestamp() + 120,
            },
            unused_token_hash
                .as_deref()
                .map(|token_hash| (id.user.as_str(), client_display_name(&client), token_hash)),
        )
        .await?;
        let c = repo.agent(&id.user, &connection).await?;
        repo.audit_agent(&c, "oauth_approve", "ok", None, None, None)
            .await?;
        redirect.query_pairs_mut().append_pair("code", &code);
    } else if input.decision == "deny" {
        redirect
            .query_pairs_mut()
            .append_pair("error", "access_denied");
    } else {
        return Err(invalid());
    }
    redirect
        .query_pairs_mut()
        .append_pair("state", &a.state)
        .append_pair("iss", &app.config.origin);
    Ok(Redirect::to(redirect.as_str()).into_response())
}
#[derive(Deserialize)]
pub(crate) struct TokenRequest {
    grant_type: String,
    client_id: String,
    resource: String,
    code: Option<String>,
    code_verifier: Option<String>,
    redirect_uri: Option<String>,
    refresh_token: Option<String>,
}
pub(crate) async fn token(State(app): State<App>, Form(input): Form<TokenRequest>) -> Response {
    match exchange(&app, input).await {
        Ok(v) => Json(v).into_response(),
        Err(e) if e.0 == StatusCode::BAD_REQUEST => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"invalid_grant"})),
        )
            .into_response(),
        Err(e) => e.into_response(),
    }
}
async fn exchange(app: &App, input: TokenRequest) -> Result<Value> {
    let at = Utc::now().timestamp();
    if input.resource != format!("{}/mcp", app.config.origin) || input.client_id.len() > 256 {
        return Err(invalid());
    }
    let access = random();
    let refresh = random();
    let mut t = AgentOAuthToken {
        access_hash: hash(&access),
        refresh_hash: hash(&refresh),
        connection: String::new(),
        client: input.client_id,
        resource: input.resource,
        expires: at + 3600,
        refresh_expires: at + 30 * 86400,
    };
    if input.grant_type == "authorization_code" {
        let verifier = input.code_verifier.ok_or_else(invalid)?;
        if !(43..=128).contains(&verifier.len())
            || !verifier
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-._~".contains(&c))
        {
            return Err(invalid());
        }
        let c = AgentOAuthCode {
            hash: hash(&input.code.ok_or_else(invalid)?),
            connection: String::new(),
            client: t.client.clone(),
            redirect: input.redirect_uri.ok_or_else(invalid)?,
            challenge: hash(&verifier),
            resource: t.resource.clone(),
            expires: at,
        };
        t.connection = app
            .store
            .control_repository()
            .consume_agent_code(&c, at)
            .await?
            .ok_or_else(invalid)?;
        app.store
            .control_repository()
            .create_agent_token(&t)
            .await?;
    } else if input.grant_type == "refresh_token" {
        if !app
            .store
            .control_repository()
            .rotate_agent_token(&hash(&input.refresh_token.ok_or_else(invalid)?), &t, at)
            .await?
        {
            return Err(invalid());
        }
    } else {
        return Err(invalid());
    }
    Ok(
        json!({"access_token":access,"token_type":"Bearer","expires_in":3600,"refresh_token":refresh,"scope":"mail"}),
    )
}
