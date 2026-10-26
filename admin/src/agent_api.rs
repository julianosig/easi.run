//! The agent API: plain HTTP and JSON under `/api/agent/`, for an agent holding a token (see `tokens`). It starts,
//! lists, describes and stops the token's own sessions and calls the driver's desktop tools on them (see `tools`),
//! through the same operations as the administrator's page (see `service`), and resumes the saved sessions (saved
//! Chrome profiles, see `profiles`) allowed to it. No viewer stream, download, takeover,
//! setting or token management is reachable this way, and every result goes through the redaction contract.

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, http::HeaderValue};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{Shared, detached, error, no_session, stopped};
use crate::app::{Features, Owner, Session};
use crate::service::{self, Start};
use crate::tokens::Denied;

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/agent/sessions", get(list).post(create))
        .route("/api/agent/sessions/{id}", get(status).delete(stop))
        .route(
            "/api/agent/sessions/{id}/tools/{tool}",
            post(call).layer(DefaultBodyLimit::max(crate::tools::BODY_LIMIT)),
        )
        .route("/api/agent/tools", get(tools))
        .route("/api/agent/saved-sessions", get(saved_sessions))
}

/// A refused bearer request, as HTTP.
pub fn denied(why: Denied) -> Response {
    match why {
        Denied::Demo => error(StatusCode::FORBIDDEN, "not available in the demo"),
        Denied::Origin => error(StatusCode::FORBIDDEN, "forbidden"),
        Denied::Unauthenticated => unauthenticated(),
    }
}

/// Why a tool call's body could not be read: 413 past [`crate::tools::BODY_LIMIT`].
pub fn unread(rejection: &BytesRejection) -> (StatusCode, String) {
    let status = rejection.status();
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        (status, format!("the request body is larger than {} bytes", crate::tools::BODY_LIMIT))
    } else {
        (status, rejection.body_text())
    }
}

pub fn unauthenticated() -> Response {
    let mut response = error(StatusCode::UNAUTHORIZED, "a valid agent token is required");
    response.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    response
}

/// The live token a request presents (see `tokens::check`), checked before the request's body is read.
struct Bearer(Owner);

impl axum::extract::FromRequestParts<Shared> for Bearer {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut axum::http::request::Parts, app: &Shared) -> Result<Self, Response> {
        crate::tokens::check(app, &parts.headers).map(|id| Bearer(Owner::Token(id))).map_err(denied)
    }
}

/// A session as its token sees it: its description, the saved session it resumed as `saved_session` (the
/// administrator's page calls it `profile`), and where the administrator watches it.
pub fn shown(app: &Shared, session: &Session) -> Value {
    let mut shown = service::describe(session);
    if let Some(described) = shown.as_object_mut() {
        let resumed = described.remove("profile").unwrap_or(Value::Null);
        described.insert("saved_session".into(), resumed);
    }
    shown["watch_url"] = json!(service::watch_url(app, &session.id));
    shown
}

/// Every session `who` reaches, each as [`shown`]: what both the agent API and the MCP `list_sessions` answer.
pub fn listed(app: &Shared, who: Owner) -> Vec<Value> {
    service::sessions_of(app, who).iter().map(|s| shown(app, s)).collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    pub url: String,
    /// The name of a configured egress; none leaves directly.
    #[serde(default)]
    pub egress: Option<String>,
    /// The id of a saved session this token may resume (see `saved_sessions`).
    #[serde(default)]
    pub resume: Option<String>,
}

/// Starts a session for a token: a default one (no DevTools, every viewer feature), as it asked, through the egress it
/// names, if any.
pub async fn start(app: &Shared, who: Owner, body: &Create) -> Result<Value, service::Refused> {
    let url = service::target(&body.url).map_err(|why| (StatusCode::BAD_REQUEST, why))?;
    let egress = body.egress.clone().map(service::EgressChoice::Name);
    let start = Start {
        id: service::session_id(),
        url,
        devtools: false,
        egress,
        kiosk: false,
        lane: None,
        owner: who,
        features: Features::default(),
        profile: body.resume.clone(),
        size: None,
    };
    let session = service::start(app, start).await?;
    Ok(shown(app, &session))
}

async fn create(State(app): State<Shared>, Bearer(who): Bearer, Json(body): Json<Create>) -> Response {
    detached(async move {
        match start(&app, who, &body).await {
            Ok(session) => Json(session).into_response(),
            Err(refused) => refused.response(),
        }
    })
    .await
}

async fn list(State(app): State<Shared>, Bearer(who): Bearer) -> Response {
    Json(json!({"sessions": listed(&app, who)})).into_response()
}

async fn status(State(app): State<Shared>, Bearer(who): Bearer, Path(id): Path<String>) -> Response {
    match app.session_for(who, &id) {
        Some(session) => Json(shown(&app, &session)).into_response(),
        None => no_session(),
    }
}

async fn stop(State(app): State<Shared>, Bearer(who): Bearer, Path(id): Path<String>) -> Response {
    let Some(session) = app.session_for(who, &id) else { return no_session() };
    stopped(app, session).await
}

/// The saved sessions `who` may resume: what both the agent API and the MCP `list_saved_sessions` answer.
pub fn resumable(app: &Shared, who: Owner) -> Value {
    let unlocked = app.auth.lock().unwrap().kek().is_some();
    json!({"saved_sessions": app.profiles.lock().unwrap().list(who), "unlocked": unlocked})
}

async fn saved_sessions(State(app): State<Shared>, Bearer(who): Bearer) -> Response {
    Json(resumable(&app, who)).into_response()
}

async fn tools(Bearer(_): Bearer) -> Response {
    Json(json!({"tools": crate::tools::driver()})).into_response()
}

/// One tool call: the request body is the tool's arguments (none, or a JSON object), the response its MCP result. The
/// body is read only for a valid token: [`Bearer`] runs before it.
async fn call(
    State(app): State<Shared>,
    Bearer(who): Bearer,
    Path((id, tool)): Path<(String, String)>,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let body = match body {
        Ok(body) => body,
        Err(rejection) => {
            let (status, why) = unread(&rejection);
            return error(status, why);
        }
    };
    if !crate::tools::known(&tool) {
        return error(StatusCode::NOT_FOUND, "no such tool; GET /api/agent/tools lists them");
    }
    let arguments = if body.is_empty() { Ok(json!({})) } else { serde_json::from_slice::<Value>(&body) };
    let Ok(arguments @ Value::Object(_)) = arguments else {
        return error(StatusCode::BAD_REQUEST, "the body is the tool's arguments: a JSON object");
    };
    let Some(session) = app.session_for(who, &id) else { return no_session() };
    let (outcome, held) = service::call(&app, who, &session, &tool, arguments).await;
    crate::api::called(outcome, held)
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    use super::*;

    /// A bearer request through the whole router, answered as status and JSON body.
    async fn send(app: &Shared, method: &str, token: &str, path: &str, body: String) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json")
            .body(Body::from(body))
            .unwrap();
        let response = crate::api::router(app.clone()).oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn a_token_resumes_only_the_saved_sessions_that_allow_it_and_learns_of_no_other() {
        let app = crate::app::test_app(false);
        let (listed, token) = app.tokens.lock().unwrap().create("t").unwrap();
        let token_id = listed["id"].as_str().unwrap().to_owned();
        let kek = crate::keys::derive("correct horse battery", &[1; crate::keys::SALT]);
        let id =
            app.profiles.lock().unwrap().create("Personal", Some(&kek)).unwrap()["id"].as_str().unwrap().to_owned();
        let start = json!({"url": "https://a.test/", "resume": id}).to_string();
        let listing = || async { send(&app, "GET", &token, "/api/agent/saved-sessions", String::new()).await.1 };
        let (status, refused) = send(&app, "POST", &token, "/api/agent/sessions", start.clone()).await;
        assert_eq!(
            (status, &refused["error"]),
            (StatusCode::NOT_FOUND, &json!("no such saved session")),
            "not allowed"
        );
        let unknown = json!({"url": "https://a.test/", "resume": "0123456789abcdef"}).to_string();
        let (status, refused_too) = send(&app, "POST", &token, "/api/agent/sessions", unknown).await;
        assert_eq!(
            (status, &refused_too),
            (StatusCode::NOT_FOUND, &refused),
            "the same answer as one that is not there"
        );
        assert_eq!(listing().await["saved_sessions"], json!([]));
        app.profiles.lock().unwrap().update(&id, None, Some(&[token_id])).unwrap();
        assert_eq!(listing().await["saved_sessions"], json!([{"id": id, "name": "Personal", "running": false}]));
        let (status, _) = send(&app, "POST", &token, "/api/agent/sessions", start).await;
        assert_eq!(status, StatusCode::LOCKED, "allowed, but no login since the start");
    }

    #[tokio::test]
    async fn a_tool_call_fits_the_largest_script_and_a_larger_body_is_answered_in_kind() {
        let app = crate::app::test_app(false);
        let token = app.tokens.lock().unwrap().create("t").unwrap().1;
        // run_script's longest script, every code point written as a surrogate pair's two escapes.
        let script = r"😀".repeat(65_536);
        let call = format!(r#"{{"script":"{script}","timeout_ms":120000,"max_calls":500}}"#);
        let (status, body) =
            send(&app, "POST", &token, "/api/agent/sessions/none/tools/run_script", call.clone()).await;
        assert_eq!((status, &body["error"]), (StatusCode::NOT_FOUND, &json!("no such session")), "read in full");
        let mcp = format!(
            r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"run_script","arguments":{{"session":"none","arguments":{call}}}}}}}"#
        );
        let (status, body) = send(&app, "POST", &token, "/mcp", mcp).await;
        assert_ne!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
        assert_eq!(body["id"], 3, "read in full: {body}");

        let larger = format!(r#"{{"script":"{}"}}"#, "a".repeat(crate::tools::BODY_LIMIT));
        let (status, body) =
            send(&app, "POST", &token, "/api/agent/sessions/none/tools/run_script", larger.clone()).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(body["error"].as_str().is_some_and(|e| e.contains("larger than")), "{body}");
        let (status, body) = send(&app, "POST", &token, "/mcp", larger.clone()).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!((body["jsonrpc"].as_str(), body["error"]["code"].as_i64()), (Some("2.0"), Some(-32600)), "{body}");
        // Without a valid token, the body is never read.
        let wrong = format!("easi_{}", "A".repeat(43));
        for path in ["/api/agent/sessions/none/tools/run_script", "/mcp"] {
            assert_eq!(send(&app, "POST", &wrong, path, larger.clone()).await.0, StatusCode::UNAUTHORIZED, "{path}");
        }
    }
}
