//! The MCP endpoint, `POST /mcp`: the agent API's operations as MCP tools, for an agent holding a token (see `tokens`).
//! Streamable HTTP with JSON responses only (no SSE stream, no GET), stateless, serving both eras of the protocol:
//!
//! - Modern, `2026-07-28`: every request carries its version and the client's capabilities in `params._meta`, and its
//!   `MCP-Protocol-Version`, `Mcp-Method` and (for `tools/call`) `Mcp-Name` headers must match the body (`-32020`
//!   otherwise); an unsupported version is refused with `-32022` and the versions served. Methods: `server/discover`,
//!   `tools/list`, `tools/call`.
//! - Legacy, `2025-11-25` and `2025-06-18`, for clients that open with `initialize`: answered without a session id
//!   (this server keeps no session state), then `tools/list`, `tools/call` and `ping` with the negotiated version in
//!   `MCP-Protocol-Version`.
//!
//! The tools: `start_session`, `stop_session`, `list_sessions`, `session_status`, and each driver tool (see `tools`)
//! taking `{session, arguments}`, its own arguments under `arguments`. They go through the same operations as the agent
//! API (see `service`): a tool that fails, the agent blocked while a person holds control included, answers a result
//! with `isError`.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{FromRequest, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use serde_json::{Map, Value, json};

use crate::agent_api::{self, Create};
use crate::api::Shared;
use crate::app::Owner;
use crate::service::{self, Called};
use crate::tokens::Denied;

const MODERN: &str = "2026-07-28";
/// The legacy versions served, newest first.
const LEGACY: [&str; 2] = ["2025-11-25", "2025-06-18"];
const SUPPORTED: [&str; 3] = [MODERN, LEGACY[0], LEGACY[1]];
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const META_SERVER: &str = "io.modelcontextprotocol/serverInfo";
/// How long a client may keep the discovery result and the tool list: they change only with the server.
const TTL_MS: u64 = 300_000;
/// The version a legacy request without `MCP-Protocol-Version` is taken to speak (2025-11-25 transports).
const UNSTATED: &str = "2025-03-26";

const INSTRUCTIONS: &str = "easi.run runs real Chrome sessions that a person can watch and take over. Start one with \
start_session: it returns the session id and a watch_url where the administrator watches it. Pass that id as \
`session` to the desktop tools, with the tool's own arguments under `arguments`. While a person holds control of a \
session, its desktop tools answer an error saying so: wait, then try again. Stop a session you no longer need with \
stop_session.";

/// Why a request is answered with a JSON-RPC error, or with no JSON-RPC at all (a token revoked mid-call).
enum Fault {
    Rpc { status: StatusCode, code: i64, message: String, data: Option<Value> },
    Revoked,
}

fn fault(status: StatusCode, code: i64, message: impl ToString) -> Fault {
    Fault::Rpc { status, code, message: message.to_string(), data: None }
}

fn header_mismatch(message: impl ToString) -> Fault {
    fault(StatusCode::BAD_REQUEST, -32020, message)
}

fn invalid_params(message: impl ToString) -> Fault {
    fault(StatusCode::BAD_REQUEST, -32602, message)
}

fn unsupported(requested: &str) -> Fault {
    Fault::Rpc {
        status: StatusCode::BAD_REQUEST,
        code: -32022,
        message: "Unsupported protocol version".into(),
        data: Some(json!({"supported": SUPPORTED, "requested": requested})),
    }
}

fn not_found(method: &str) -> Fault {
    fault(StatusCode::NOT_FOUND, -32601, format!("Method not found: {method}"))
}

fn respond(id: Option<&Value>, outcome: Result<Value, Fault>) -> Response {
    match outcome {
        Ok(result) => Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response(),
        Err(Fault::Revoked) => agent_api::unauthenticated(),
        Err(Fault::Rpc { status, code, message, data }) => {
            let mut error = json!({"code": code, "message": message});
            if let Some(data) = data {
                error["data"] = data;
            }
            let mut body = json!({"jsonrpc": "2.0", "error": error});
            if let Some(id) = id {
                body["id"] = id.clone();
            }
            (status, Json(body)).into_response()
        }
    }
}

/// One POST: the token, the media types, the message, then the era it speaks. The body is read only for a valid token.
pub async fn endpoint(State(app): State<Shared>, headers: HeaderMap, request: Request) -> Response {
    let who = match crate::tokens::check(&app, &headers) {
        Ok(id) => Owner::Token(id),
        Err(Denied::Origin) => return respond(None, Err(fault(StatusCode::FORBIDDEN, -32600, "Forbidden origin"))),
        Err(why) => return agent_api::denied(why),
    };
    let media = |name| headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_ascii_lowercase);
    let json_body =
        media(header::CONTENT_TYPE).is_some_and(|t| t.split(';').next().unwrap().trim() == "application/json");
    if !json_body {
        let refused = fault(StatusCode::UNSUPPORTED_MEDIA_TYPE, -32600, "Content-Type must be application/json");
        return respond(None, Err(refused));
    }
    // Every answer is one JSON object.
    // Every Accept field, as one list.
    let accept: Vec<&str> = headers.get_all(header::ACCEPT).iter().filter_map(|v| v.to_str().ok()).collect();
    let accepted = accept.is_empty() || accepts_json(&accept.join(",").to_ascii_lowercase());
    if !accepted {
        let refused = fault(StatusCode::NOT_ACCEPTABLE, -32600, "Accept must list application/json");
        return respond(None, Err(refused));
    }
    let body = match Bytes::from_request(request, &()).await {
        Ok(body) => body,
        Err(rejection) => {
            let (status, why) = agent_api::unread(&rejection);
            return respond(None, Err(fault(status, -32600, format!("Invalid Request: {why}"))));
        }
    };
    let message: Value = match serde_json::from_slice(&body) {
        Ok(message) => message,
        Err(_) => return respond(None, Err(fault(StatusCode::BAD_REQUEST, -32700, "Parse error"))),
    };
    let Value::Object(message) = message else {
        let refused = fault(StatusCode::BAD_REQUEST, -32600, "Invalid Request: one JSON-RPC request or notification");
        return respond(None, Err(refused));
    };
    // Every error from here on carries the id, once it can be read.
    let id = message.get("id");
    let readable = id.filter(|id| id.is_string() || id.is_i64() || id.is_u64());
    let invalid =
        |why: &str| respond(readable, Err(fault(StatusCode::BAD_REQUEST, -32600, format!("Invalid Request: {why}"))));
    if id.is_some() && readable.is_none() {
        return invalid("the id must be a string or an integer");
    }
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return invalid("jsonrpc must be \"2.0\"");
    }
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return invalid("a request or notification needs a method");
    };
    let params = match message.get("params") {
        None => Map::new(),
        Some(Value::Object(params)) => params.clone(),
        Some(_) => return invalid("params must be an object"),
    };
    // A notification needs no answer, whatever it says.
    let Some(id) = id else { return StatusCode::ACCEPTED.into_response() };
    let header_version = headers.get("mcp-protocol-version").map(|v| v.to_str().unwrap_or_default().to_owned());
    let meta = params.get("_meta").and_then(Value::as_object);
    let outcome = if meta.is_some_and(|m| m.contains_key(META_VERSION)) || header_version.as_deref() == Some(MODERN) {
        modern(&app, who, &headers, method, &params).await
    } else if method == "initialize" {
        Ok(initialize(&params))
    } else {
        legacy(&app, who, header_version.as_deref(), method, &params).await
    };
    respond(Some(id), outcome)
}

/// Whether an `Accept` header (lowercased) admits `application/json`: the most specific range that matches it must have
/// a quality above zero.
fn accepts_json(accept: &str) -> bool {
    let mut best: Option<(u8, bool)> = None;
    for range in accept.split(',') {
        let mut parts = range.split(';').map(str::trim);
        let specificity = match parts.next().unwrap_or_default() {
            "application/json" => 2,
            "application/*" => 1,
            "*/*" => 0,
            _ => continue,
        };
        let quality = parts.find_map(|p| p.strip_prefix("q=")).map_or(Some(1.0), |q| q.parse::<f32>().ok());
        if best.is_none_or(|(s, _)| specificity > s) {
            best = Some((specificity, quality.is_some_and(|q| q > 0.0)));
        }
    }
    best.is_some_and(|(_, acceptable)| acceptable)
}

/// A modern request: its metadata and headers checked, then served statelessly.
async fn modern(
    app: &Shared,
    who: Owner,
    headers: &HeaderMap,
    method: &str,
    params: &Map<String, Value>,
) -> Result<Value, Fault> {
    let meta = params.get("_meta").and_then(Value::as_object);
    let Some(version) = meta.and_then(|m| m.get(META_VERSION)).and_then(Value::as_str) else {
        return Err(invalid_params(format!("_meta[\"{META_VERSION}\"] is required")));
    };
    let header = |name: &str| {
        headers.get(name).map(|v| v.to_str().map_err(|_| header_mismatch(format!("{name} holds invalid characters"))))
    };
    match header("mcp-protocol-version").transpose()? {
        Some(stated) if stated == version => {}
        Some(stated) => {
            return Err(header_mismatch(format!(
                "Header mismatch: MCP-Protocol-Version header value '{stated}' does not match body value '{version}'"
            )));
        }
        None => return Err(header_mismatch("Header mismatch: the MCP-Protocol-Version header is required")),
    }
    if version != MODERN {
        return Err(unsupported(version));
    }
    match header("mcp-method").transpose()? {
        Some(stated) if stated == method => {}
        Some(stated) => {
            return Err(header_mismatch(format!(
                "Header mismatch: Mcp-Method header value '{stated}' does not match body value '{method}'"
            )));
        }
        None => return Err(header_mismatch("Header mismatch: the Mcp-Method header is required")),
    }
    if !meta.and_then(|m| m.get(META_CAPABILITIES)).is_some_and(Value::is_object) {
        return Err(invalid_params(format!("_meta[\"{META_CAPABILITIES}\"] is required")));
    }
    let mut result = match method {
        "server/discover" => json!({
            "supportedVersions": SUPPORTED,
            "capabilities": {"tools": {}},
            "instructions": INSTRUCTIONS,
            "ttlMs": TTL_MS,
            "cacheScope": "public",
        }),
        "tools/list" => {
            if params.contains_key("cursor") {
                return Err(invalid_params("Invalid cursor: the tool list has one page"));
            }
            json!({"tools": tool_list(), "ttlMs": TTL_MS, "cacheScope": "public"})
        }
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Err(invalid_params("params.name must be a string"));
            };
            match header("mcp-name").transpose()? {
                Some(stated) => {
                    let decoded =
                        sentinel(stated).ok_or_else(|| header_mismatch("Mcp-Name holds a malformed Base64 value"))?;
                    if decoded != name {
                        return Err(header_mismatch(format!(
                            "Header mismatch: Mcp-Name header value '{stated}' does not match body value '{name}'"
                        )));
                    }
                }
                None => return Err(header_mismatch("Header mismatch: the Mcp-Name header is required")),
            }
            call_tool(app, who, params).await?
        }
        _ => return Err(not_found(method)),
    };
    result["resultType"] = json!("complete");
    let result_meta = result.as_object_mut().expect("an object").entry("_meta").or_insert_with(|| json!({}));
    if let Some(m) = result_meta.as_object_mut() {
        m.insert(META_SERVER.into(), server_info());
    }
    Ok(result)
}

/// A header value as the client meant it: the Base64 sentinel form (`=?base64?...?=`) decoded.
fn sentinel(value: &str) -> Option<String> {
    match value.strip_prefix("=?base64?").and_then(|v| v.strip_suffix("?=")) {
        Some(encoded) => {
            let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
            String::from_utf8(bytes).ok()
        }
        None => Some(value.to_owned()),
    }
}

fn server_info() -> Value {
    json!({"name": "easi.run", "version": env!("CARGO_PKG_VERSION")})
}

/// A legacy `initialize`: the client's version when it is one served, else the newest legacy one. No session id: this
/// server keeps no state between requests.
fn initialize(params: &Map<String, Value>) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
    let version = LEGACY.iter().find(|v| **v == asked).copied().unwrap_or(LEGACY[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {}},
        "serverInfo": server_info(),
        "instructions": INSTRUCTIONS,
    })
}

/// A legacy request after `initialize`, at the version its `MCP-Protocol-Version` header states.
async fn legacy(
    app: &Shared,
    who: Owner,
    version: Option<&str>,
    method: &str,
    params: &Map<String, Value>,
) -> Result<Value, Fault> {
    let version = version.unwrap_or(UNSTATED);
    if !LEGACY.contains(&version) {
        return Err(unsupported(version));
    }
    let unknown = || fault(StatusCode::OK, -32601, format!("Method not found: {method}"));
    match method {
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tool_list()})),
        "tools/call" => call_tool(app, who, params).await.map_err(|f| match f {
            // Legacy clients read a protocol error from the JSON-RPC body; a 400 could end their session.
            Fault::Rpc { code, message, data, .. } => Fault::Rpc { status: StatusCode::OK, code, message, data },
            revoked => revoked,
        }),
        _ => Err(unknown()),
    }
}

/// An object schema with these properties, nothing else.
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false})
}

const SESSION_ARG: &str = "The session id start_session returned.";

/// The tools: the session lifecycle, then each driver tool wrapped to take its session.
fn tool_list() -> Vec<Value> {
    let session = json!({"type": "string", "description": SESSION_ARG});
    let read_only =
        json!({"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false});
    let mut tools = vec![
        json!({
            "name": "start_session",
            "description": "Start a Chrome session at a URL. Returns its id, which every other tool takes as `session`, \
                and a watch_url where the administrator watches it and can take over. A person who takes over pauses \
                the desktop tools until they give control back.",
            "inputSchema": object(json!({
                "url": {"type": "string", "description": "The http(s) page to open, at most 2048 bytes."},
                "egress": {"type": "string", "description": "The name of one of the installation's egresses to leave through; directly when left out."},
            }), &["url"]),
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": true},
        }),
        json!({
            "name": "stop_session",
            "description": "Stop a session and delete its browser.",
            "inputSchema": object(json!({"session": session}), &["session"]),
            "annotations": {"readOnlyHint": false, "destructiveHint": true, "idempotentHint": true, "openWorldHint": false},
        }),
        json!({
            "name": "list_sessions",
            "description": "List the sessions started with this token: their ids, URLs, whether a person holds control (`controlled`), and why one failed.",
            "inputSchema": object(json!({}), &[]),
            "annotations": read_only,
        }),
        json!({
            "name": "session_status",
            "description": "Describe one session: its URL, whether a person holds control (`controlled`), why it failed, and its watch_url.",
            "inputSchema": object(json!({"session": session}), &["session"]),
            "annotations": read_only,
        }),
    ];
    for tool in crate::tools::driver() {
        let mut wrapped = tool.clone();
        let description = tool["description"].as_str().unwrap_or_default();
        wrapped["description"] = json!(format!(
            "{description}\n\nOn an easi.run session: pass `session`, and this tool's own arguments under `arguments`."
        ));
        wrapped["inputSchema"] = object(json!({"session": session, "arguments": tool["inputSchema"]}), &["session"]);
        tools.push(wrapped);
    }
    tools
}

/// A tool's result for the caller: its JSON as text, and as structured content.
fn result(value: Value) -> Value {
    json!({"content": [{"type": "text", "text": value.to_string()}], "structuredContent": value, "isError": false})
}

/// A tool that failed, said so the model can act on it.
fn failed(why: impl ToString) -> Value {
    json!({"content": [{"type": "text", "text": why.to_string()}], "isError": true})
}

/// One `tools/call`. Malformed calls and unknown tools are protocol errors; everything a tool itself refuses is a
/// result with `isError`.
async fn call_tool(app: &Shared, who: Owner, params: &Map<String, Value>) -> Result<Value, Fault> {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return Err(invalid_params("params.name must be a string"));
    };
    let arguments = match params.get("arguments") {
        None => Map::new(),
        Some(Value::Object(arguments)) => arguments.clone(),
        Some(_) => return Err(invalid_params("params.arguments must be an object")),
    };
    let session_arg = || arguments.get("session").and_then(Value::as_str).map(str::to_owned);
    let reached = |id: Option<String>| id.and_then(|id| app.session_for(who, &id));
    match name {
        "start_session" => {
            let create: Create = match serde_json::from_value(Value::Object(arguments.clone())) {
                Ok(create) => create,
                Err(why) => return Ok(failed(format!("start_session takes {{url, egress?}}: {why}"))),
            };
            let shared = app.clone();
            // Its own task: a client that disconnects mid-way cannot leave a place reserved or a session unlisted.
            let started = tokio::spawn(async move { agent_api::start(&shared, who, &create).await }).await;
            Ok(match started {
                Ok(Ok(session)) => result(session),
                Ok(Err((StatusCode::UNAUTHORIZED, _))) => return Err(Fault::Revoked),
                Ok(Err((_, why))) => failed(why),
                Err(_) => failed("the start failed"),
            })
        }
        "stop_session" => {
            let Some(session) = reached(session_arg()) else { return Ok(failed("no such session")) };
            let shared = app.clone();
            let stopped = tokio::spawn(async move { service::stop(&shared, &session).await }).await;
            Ok(match stopped {
                Ok(Ok(())) => result(json!({"stopped": true})),
                Ok(Err(why)) => failed(format!("stopping the session failed: {why}")),
                Err(_) => failed("stopping the session failed"),
            })
        }
        "list_sessions" => {
            let sessions: Vec<Value> =
                service::sessions_of(app, who).iter().map(|s| agent_api::shown(app, s)).collect();
            Ok(result(json!({"sessions": sessions})))
        }
        "session_status" => Ok(match reached(session_arg()) {
            Some(session) => result(agent_api::shown(app, &session)),
            None => failed("no such session"),
        }),
        tool if crate::tools::known(tool) => {
            let Some(session) = reached(session_arg()) else { return Ok(failed("no such session")) };
            let tool_arguments = match arguments.get("arguments") {
                None => json!({}),
                Some(value @ Value::Object(_)) => value.clone(),
                Some(_) => return Ok(failed("`arguments` must be an object: the tool's own arguments")),
            };
            Ok(match service::call(app, who, &session, tool, tool_arguments).await {
                Called::Done(result) if result.is_object() => result,
                Called::Done(_) => failed("the agent answered something other than a tool result"),
                Called::Controlled => failed(
                    "A person holds control of this session in the viewer, so its desktop tools are paused. Wait, then \
                     try again once they give it back (session_status shows `controlled`).",
                ),
                Called::Stopped => failed("the session was stopped after a failure; start another"),
                Called::Revoked => return Err(Fault::Revoked),
                Called::Busy => failed("This session's agent is busy with other calls; try again shortly."),
                Called::Failed(why) => failed(why),
            })
        }
        _ => Err(fault(StatusCode::OK, -32602, format!("Unknown tool: {name}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Client {
        app: crate::app::TestApp,
        token: String,
    }

    fn client(demo: bool) -> Client {
        let app = crate::app::test_app(demo);
        let token =
            if demo { format!("easi_{}", "A".repeat(43)) } else { app.tokens.lock().unwrap().create("t").unwrap().1 };
        Client { app, token }
    }

    impl Client {
        /// A POST with these headers (the token, JSON and the right Accept unless overridden), answered as status, headers
        /// and JSON body (null when empty).
        async fn post(&self, extra: &[(&str, &str)], body: Value) -> (StatusCode, HeaderMap, Value) {
            let mut headers = HeaderMap::new();
            headers.insert(header::AUTHORIZATION, format!("Bearer {}", self.token).parse().unwrap());
            headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
            headers.insert(header::ACCEPT, "application/json, text/event-stream".parse().unwrap());
            for (name, value) in extra {
                let name = header::HeaderName::from_bytes(name.as_bytes()).unwrap();
                if value.is_empty() {
                    headers.remove(&name);
                } else {
                    headers.insert(name, value.parse().unwrap());
                }
            }
            let response = endpoint(State(self.app.clone()), headers, Request::new(body.to_string().into())).await;
            let status = response.status();
            let headers = response.headers().clone();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let json = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
            (status, headers, json)
        }

        /// A modern request with matching headers.
        async fn modern(&self, method: &str, mut params: Value) -> (StatusCode, Value) {
            params["_meta"] = json!({META_VERSION: MODERN, META_CAPABILITIES: {}, "io.modelcontextprotocol/clientInfo": {"name": "test", "version": "1"}});
            let name = params["name"].as_str().unwrap_or_default().to_owned();
            let mut headers = vec![("mcp-protocol-version", MODERN), ("mcp-method", method)];
            if !name.is_empty() {
                headers.push(("mcp-name", &name));
            }
            let (status, _, body) =
                self.post(&headers, json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params})).await;
            (status, body)
        }
    }

    fn request(method: &str, params: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
    }

    fn modern_params(extra: Value) -> Value {
        let mut params = extra;
        params["_meta"] = json!({META_VERSION: MODERN, META_CAPABILITIES: {}});
        params
    }

    #[tokio::test]
    async fn discovery_and_the_tool_list_carry_what_the_modern_schema_requires() {
        let c = client(false);
        let (status, body) = c.modern("server/discover", json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["id"], 7);
        let result = &body["result"];
        assert_eq!(result["resultType"], "complete");
        assert_eq!(result["supportedVersions"], json!(SUPPORTED));
        assert!(result["capabilities"]["tools"].is_object());
        assert!(result["ttlMs"].as_u64().is_some() && result["cacheScope"] == "public");
        assert_eq!(result["_meta"][META_SERVER]["name"], "easi.run");

        let (status, body) = c.modern("tools/list", json!({})).await;
        assert_eq!(status, StatusCode::OK);
        let result = &body["result"];
        assert_eq!(result["resultType"], "complete");
        assert!(result["ttlMs"].is_u64() && result["cacheScope"] == "public");
        let tools = result["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for name in ["start_session", "stop_session", "list_sessions", "session_status", "click", "get_desktop_state"] {
            assert!(names.contains(&name), "{name}");
        }
        assert!(!names.iter().any(|n| n.starts_with("browser_")));
        let click = tools.iter().find(|t| t["name"] == "click").unwrap();
        assert_eq!(click["inputSchema"]["required"], json!(["session"]));
        let driver = crate::tools::driver().iter().find(|t| t["name"] == "click").unwrap();
        assert_eq!(click["inputSchema"]["properties"]["arguments"], driver["inputSchema"], "the driver's schema, kept");
    }

    #[tokio::test]
    async fn modern_headers_must_match_the_body() {
        let c = client(false);
        let discover = request("server/discover", modern_params(json!({})));
        let cases: [&[(&str, &str)]; 4] = [
            &[("mcp-method", "server/discover")],
            &[("mcp-protocol-version", "2025-11-25"), ("mcp-method", "server/discover")],
            &[("mcp-protocol-version", MODERN)],
            &[("mcp-protocol-version", MODERN), ("mcp-method", "tools/list")],
        ];
        for headers in cases {
            let (status, _, body) = c.post(headers, discover.clone()).await;
            assert_eq!(
                (status, body["error"]["code"].as_i64()),
                (StatusCode::BAD_REQUEST, Some(-32020)),
                "{headers:?}"
            );
            assert_eq!(body["id"], 1);
        }
        // tools/call names its tool in Mcp-Name, in the Base64 sentinel form when the client chose it.
        let call = request("tools/call", modern_params(json!({"name": "list_sessions", "arguments": {}})));
        let base = [("mcp-protocol-version", MODERN), ("mcp-method", "tools/call")];
        let (status, _, body) = c.post(&base, call.clone()).await;
        assert_eq!((status, body["error"]["code"].as_i64()), (StatusCode::BAD_REQUEST, Some(-32020)), "no Mcp-Name");
        let (status, _, body) = c.post(&[base[0], base[1], ("mcp-name", "stop_session")], call.clone()).await;
        assert_eq!((status, body["error"]["code"].as_i64()), (StatusCode::BAD_REQUEST, Some(-32020)), "another name");
        let encoded = format!("=?base64?{}?=", base64::engine::general_purpose::STANDARD.encode("list_sessions"));
        let (status, _, body) = c.post(&[base[0], base[1], ("mcp-name", &encoded)], call.clone()).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["result"]["structuredContent"]["sessions"], json!([]));
        assert_eq!(body["result"]["resultType"], "complete");
        let (status, _, body) = c.post(&[base[0], base[1], ("mcp-name", "=?base64?!!?=")], call).await;
        assert_eq!((status, body["error"]["code"].as_i64()), (StatusCode::BAD_REQUEST, Some(-32020)));
    }

    #[tokio::test]
    async fn versions_capabilities_and_methods_are_checked() {
        let c = client(false);
        let mut old = request("server/discover", modern_params(json!({})));
        old["params"]["_meta"][META_VERSION] = json!("1900-01-01");
        let (status, _, body) =
            c.post(&[("mcp-protocol-version", "1900-01-01"), ("mcp-method", "server/discover")], old).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], -32022);
        assert_eq!(body["error"]["data"], json!({"supported": SUPPORTED, "requested": "1900-01-01"}));

        let mut bare = request("server/discover", modern_params(json!({})));
        bare["params"]["_meta"].as_object_mut().unwrap().remove(META_CAPABILITIES);
        let headers = [("mcp-protocol-version", MODERN), ("mcp-method", "server/discover")];
        let (status, _, body) = c.post(&headers, bare).await;
        assert_eq!((status, body["error"]["code"].as_i64()), (StatusCode::BAD_REQUEST, Some(-32602)));

        // ping left the protocol in 2026-07-28; so did initialize for modern clients.
        for method in ["ping", "resources/list", "nothing/here"] {
            let (status, body) = c.modern(method, json!({})).await;
            assert_eq!((status, body["error"]["code"].as_i64()), (StatusCode::NOT_FOUND, Some(-32601)), "{method}");
        }
        let (status, body) = c.modern("tools/call", json!({"name": "no_such_tool", "arguments": {}})).await;
        assert_eq!((status, body["error"]["code"].as_i64()), (StatusCode::OK, Some(-32602)));
    }

    #[tokio::test]
    async fn a_legacy_client_initializes_without_a_session_and_then_states_its_version() {
        let c = client(false);
        let init = request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "old", "version": "1"}}),
        );
        let (status, headers, body) = c.post(&[], init).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!headers.contains_key("mcp-session-id"), "stateless: no session id");
        assert_eq!(body["result"]["protocolVersion"], "2025-06-18");
        assert!(
            body["result"]["capabilities"]["tools"].is_object() && body["result"]["serverInfo"]["name"] == "easi.run"
        );
        let newer = request("initialize", json!({"protocolVersion": "2099-01-01", "capabilities": {}}));
        assert_eq!(c.post(&[], newer).await.2["result"]["protocolVersion"], LEGACY[0], "the newest legacy one");

        let initialized = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let (status, _, body) = c.post(&[("mcp-protocol-version", "2025-06-18")], initialized).await;
        assert_eq!((status, body), (StatusCode::ACCEPTED, Value::Null));

        let version = [("mcp-protocol-version", "2025-11-25")];
        assert_eq!(c.post(&version, request("ping", json!({}))).await.2["result"], json!({}));
        let (_, _, body) = c.post(&version, request("tools/list", json!({}))).await;
        assert!(body["result"]["tools"].as_array().unwrap().len() > 4 && body["result"].get("resultType").is_none());
        let call =
            request("tools/call", json!({"name": "click", "arguments": {"session": "000000000000", "arguments": {}}}));
        let (status, _, body) = c.post(&version, call).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["isError"], true, "a tool failure is a result: {body}");
        let (status, _, body) = c.post(&[], request("tools/list", json!({}))).await;
        assert_eq!(
            (status, body["error"]["code"].as_i64()),
            (StatusCode::BAD_REQUEST, Some(-32022)),
            "no version stated"
        );
        assert_eq!(body["error"]["data"]["requested"], UNSTATED);
    }

    #[tokio::test]
    async fn only_a_live_token_from_no_other_site_gets_in_and_only_json_is_spoken() {
        let c = client(false);
        let discover = request("server/discover", modern_params(json!({})));
        let headers = [("mcp-protocol-version", MODERN), ("mcp-method", "server/discover")];
        let (status, response_headers, _) =
            c.post(&[headers[0], headers[1], ("authorization", "")], discover.clone()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(response_headers[header::WWW_AUTHENTICATE], "Bearer");
        let wrong = format!("Bearer easi_{}", "B".repeat(43));
        assert_eq!(c.post(&[("authorization", &wrong)], discover.clone()).await.0, StatusCode::UNAUTHORIZED);
        let (status, _, body) = c.post(&[("origin", "https://evil.example")], discover.clone()).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.get("id").is_none() && body["error"]["code"].is_i64());
        let (status, _, _) =
            c.post(&[headers[0], headers[1], ("origin", "https://localhost:9443")], discover.clone()).await;
        assert_eq!(status, StatusCode::OK, "the admin's own origin may");
        assert_eq!(
            c.post(&[("content-type", "text/plain")], discover.clone()).await.0,
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
        assert_eq!(c.post(&[("accept", "text/event-stream")], discover.clone()).await.0, StatusCode::NOT_ACCEPTABLE);
        let (status, _, body) = c.post(&[], json!([discover])).await;
        assert_eq!((status, body["error"]["code"].as_i64()), (StatusCode::BAD_REQUEST, Some(-32600)), "no batches");

        let demo = client(true);
        assert_eq!(
            demo.post(&headers, request("server/discover", modern_params(json!({})))).await.0,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn an_invalid_request_s_error_carries_its_id_once_it_can_be_read() {
        let c = client(false);
        for (body, id) in [
            (json!({"jsonrpc": "2.0", "id": 7, "method": "tools/list", "params": []}), json!(7)),
            (json!({"jsonrpc": "1.0", "id": "a", "method": "tools/list"}), json!("a")),
            (json!({"jsonrpc": "2.0", "id": 8}), json!(8)),
        ] {
            let (status, _, answer) = c.post(&[], body.clone()).await;
            assert_eq!((status, answer["error"]["code"].as_i64()), (StatusCode::BAD_REQUEST, Some(-32600)), "{body}");
            assert_eq!(answer["id"], id, "{body}");
        }
        for body in [
            json!({"jsonrpc": "2.0", "id": null, "method": "tools/list"}),
            json!({"jsonrpc": "2.0", "id": {}, "method": "x"}),
        ] {
            let (_, _, answer) = c.post(&[], body.clone()).await;
            assert!(answer.get("id").is_none(), "an unreadable id is left out: {answer}");
        }
    }

    #[test]
    fn json_must_be_acceptable_by_its_most_specific_range() {
        for yes in ["application/json, text/event-stream", "*/*", "application/*;q=0.5", "*/*;q=0, application/json"] {
            assert!(accepts_json(yes), "{yes}");
        }
        for no in ["text/event-stream", "application/json;q=0", "*/*, application/json;q=0", "application/json;q=x"] {
            assert!(!accepts_json(no), "{no}");
        }
    }

    #[tokio::test]
    async fn a_token_reaches_no_other_principal_s_session() {
        let c = client(false);
        let session = |owner| {
            std::sync::Arc::new(crate::app::Session::new(
                "aaaaaaaaaaaa".into(),
                "https://a/".into(),
                false,
                None,
                owner,
                crate::app::Features::default(),
                c.app.controller.clone(),
            ))
        };
        c.app.sessions.lock().unwrap().insert("aaaaaaaaaaaa".into(), session(Owner::Admin));
        let (_, body) = c.modern("tools/call", json!({"name": "list_sessions", "arguments": {}})).await;
        assert_eq!(body["result"]["structuredContent"]["sessions"], json!([]));
        for name in ["session_status", "stop_session", "get_desktop_state"] {
            let (_, body) =
                c.modern("tools/call", json!({"name": name, "arguments": {"session": "aaaaaaaaaaaa"}})).await;
            assert_eq!(body["result"]["isError"], true, "{name}: {body}");
        }
        assert!(c.app.session("aaaaaaaaaaaa").is_some(), "never stopped");
    }
}
