//! The admin application's HTTP API and its WebSocket viewer endpoint. Every state change needs a live login, the
//! exact origin, and the login's CSRF token (see `auth`); a viewer upgrade needs the exact origin and a live login.
//! A download is fetched with the single-use ticket its holder was given, by the same login, while that viewer still
//! holds control at the ticket's epoch; the response is checked again before every piece.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Path, Query, State, WebSocketUpgrade};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use display_protocol::frame;
use serde::Deserialize;
use serde_json::{Value, json};
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::app::{App, Network, Session, Settings};
use crate::auth::{Auth, LoginKey, Refusal};
use crate::service::{Called, Start, describe, session_id, stop};
use crate::{agent, app, auth, relay, service};

pub type Shared = Arc<App>;
/// Every route, the static admin page, and the security headers every response carries.
pub fn router(app: Shared) -> Router {
    let statics = ServeDir::new(app.config.static_dir.clone());
    let static_path = app.config.static_path.clone();
    let sdk_dir = app.config.sdk_dir.clone();
    // Only the pages the demo names may frame it; nothing frames any other installation.
    let ancestors =
        if app.config.frame_ancestors.is_empty() { "'none'".to_owned() } else { app.config.frame_ancestors.join(" ") };
    let policy = format!(
        "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' blob: data:; connect-src 'self'; \
         frame-ancestors {ancestors}; base-uri 'none'; form-action 'self'"
    );
    let router = Router::new()
        .route("/api/status", get(status))
        .route("/api/enroll", post(enroll))
        .route("/api/login", post(login))
        .route("/api/demo/switch", post(demo_switch))
        .route("/api/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/egresses", get(list_egresses))
        .route("/api/sessions", get(list_sessions).post(create_session))
        .route("/api/sessions/{id}", axum::routing::delete(stop_session))
        .route("/api/sessions/{id}/control", post(take_control))
        .route("/api/sessions/{id}/release", post(give_back))
        .route("/api/sessions/{id}/settings", put(set_settings))
        .route("/api/sessions/{id}/network", put(set_network))
        .route("/api/sessions/{id}/agent", post(agent_call))
        .route("/api/sessions/{id}/stats", get(stats))
        .route("/api/sessions/{id}/files/{ticket}", get(download).head(no_head))
        .route("/ws/sessions/{id}", get(watch))
        .route("/api/tokens", get(list_tokens).post(create_token))
        .route("/api/tokens/{id}", axum::routing::delete(revoke_token))
        .merge(crate::agent_api::routes())
        .route("/mcp", post(crate::mcp::endpoint).layer(DefaultBodyLimit::max(crate::tools::BODY_LIMIT)))
        .nest_service("/sdk", ServeDir::new(sdk_dir));
    // The web client at the root, or under its own path: there `/demo` itself sends the browser on to `/demo/`, where
    // the page's relative links resolve.
    let router = if static_path == "/" {
        router.fallback_service(statics)
    } else {
        let at = static_path.clone();
        let client = Router::new()
            .fallback_service(statics)
            .layer(axum::middleware::from_fn(move |request, next| to_slash(at.clone(), request, next)));
        router.nest_service(&static_path, client)
    };
    router
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_str(&policy).expect("the origins were checked at startup"),
        ))
        .layer(SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(SetResponseHeaderLayer::overriding(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer")))
        // An installation's pages, the public demo's too, are never meant for search engines.
        .layer(SetResponseHeaderLayer::overriding(
            header::HeaderName::from_static("x-robots-tag"),
            HeaderValue::from_static("noindex"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000"),
        ))
        // The page and its script are revalidated on every load (an unchanged file costs a 304), so an update never
        // leaves an open browser speaking an older protocol than the server.
        .layer(SetResponseHeaderLayer::if_not_present(header::CACHE_CONTROL, HeaderValue::from_static("no-cache")))
        .with_state(app)
}

/// The web client's own path without its slash (`/demo`): on to `/demo/`, query kept. A directory's redirect from the
/// files below it (to its slash) keeps the path too.
async fn to_slash(at: String, request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let original = request.extensions().get::<axum::extract::OriginalUri>().map(|o| o.0.clone());
    if let Some(uri) = original.filter(|uri| uri.path() == at) {
        let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
        return axum::response::Redirect::permanent(&format!("{at}/{query}")).into_response();
    }
    let mut response = next.run(request).await;
    let location = response.headers().get(header::LOCATION).and_then(|l| l.to_str().ok());
    if let Some(prefixed) = location.filter(|l| l.starts_with('/')).map(|l| format!("{at}{l}"))
        && let Ok(value) = HeaderValue::from_str(&prefixed)
    {
        response.headers_mut().insert(header::LOCATION, value);
    }
    response
}

fn refusal(r: Refusal) -> Response {
    match r {
        Refusal::Unauthenticated => (StatusCode::UNAUTHORIZED, Json(json!({"error": "log in first"}))).into_response(),
        Refusal::Forbidden => (StatusCode::FORBIDDEN, Json(json!({"error": "forbidden"}))).into_response(),
        Refusal::BadRequest(why) => (StatusCode::BAD_REQUEST, Json(json!({"error": why}))).into_response(),
        Refusal::TooManyAttempts => {
            (StatusCode::TOO_MANY_REQUESTS, Json(json!({"error": "too many attempts; wait"}))).into_response()
        }
    }
}

/// A refusal or failure as JSON, `{"error": why}`: the one shape the administrator's page and the agent API share.
pub(crate) fn error(status: StatusCode, why: impl ToString) -> Response {
    (status, Json(json!({"error": why.to_string()}))).into_response()
}

/// An agent call refused because the session's agent is busy (see `agent::MAX_WAITING`): 503, try again shortly.
pub fn busy() -> Response {
    let mut response =
        error(StatusCode::SERVICE_UNAVAILABLE, "this session's agent is busy with other calls; try again shortly");
    response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
    response
}

/// Runs `work` as its own task: a client that disconnects mid-way cannot cancel it half done (a place left reserved,
/// a session started but never listed, an agent left blocked or dead).
pub(crate) async fn detached(work: impl Future<Output = Response> + Send + 'static) -> Response {
    tokio::spawn(work).await.unwrap_or_else(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "the request failed"))
}

fn authed(app: &App, method: &Method, headers: &HeaderMap) -> Result<LoginKey, Refusal> {
    app.auth.lock().unwrap().check(method, headers, &app.config.origin).map(|(key, _)| key)
}

/// The administrator: a live login (with the exact origin and CSRF token on a state change), outside the demo. Every
/// route that administers the installation takes it, so none can leave out the demo's refusal: each one reaches every
/// session, which a demo visitor must not. The cookie routes open to visitors check the login with `authed` and reach
/// sessions through `App::session_for`.
struct Administrator;

impl axum::extract::FromRequestParts<Shared> for Administrator {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut axum::http::request::Parts, app: &Shared) -> Result<Self, Response> {
        authed(app, &parts.method, &parts.headers).map_err(refusal)?;
        if app.config.demo.is_some() {
            return Err(not_in_the_demo());
        }
        Ok(Administrator)
    }
}

fn exact_origin(app: &App, headers: &HeaderMap) -> Result<(), Refusal> {
    if auth::origin_matches(headers, &app.config.origin) { Ok(()) } else { Err(Refusal::Forbidden) }
}

async fn status(State(app): State<Shared>) -> Response {
    let demo = app.config.demo.as_ref().map(|d| {
        let sites: Vec<&str> = d.sites.iter().map(|(name, _)| name.as_str()).collect();
        json!({"sites": sites, "seconds": d.seconds, "embedders": app.config.frame_ancestors})
    });
    Json(json!({"enrolled": app.auth.lock().unwrap().enrolled(), "demo": demo})).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Enroll {
    token: String,
    password: String,
}

async fn enroll(State(app): State<Shared>, headers: HeaderMap, Json(body): Json<Enroll>) -> Response {
    if let Err(r) = exact_origin(&app, &headers) {
        return refusal(r);
    }
    if let Err(r) = app.auth.lock().unwrap().check_enrollment(&body.token, &body.password) {
        return refusal(r);
    }
    let password = body.password;
    let hash = match tokio::task::spawn_blocking(move || Auth::hash_password(&password)).await {
        Ok(Ok(hash)) => hash,
        Ok(Err(r)) => return refusal(r),
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    match app.auth.lock().unwrap().complete_enrollment(&body.token, hash) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(r) => refusal(r),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginBody {
    /// Absent in the demo, which has none.
    #[serde(default)]
    password: String,
    /// The demo a visitor picked (the demo only).
    #[serde(default)]
    demo: Option<String>,
    /// A start the page made on its own (the next visit after time ran out): it waits for a visit this client has open
    /// elsewhere to end, rather than closing it as a visitor's own start does.
    #[serde(default)]
    wait: bool,
}

async fn login(State(app): State<Shared>, headers: HeaderMap, Json(body): Json<LoginBody>) -> Response {
    if let Err(r) = exact_origin(&app, &headers) {
        return refusal(r);
    }
    if app.config.demo.is_some() {
        return detached(demo_login(app.clone(), headers, body)).await;
    }
    let hash = match app.auth.lock().unwrap().begin_login() {
        Ok(hash) => hash,
        Err(r) => return refusal(r),
    };
    // The check and its bookkeeping run together on a blocking thread, which finishes even if this request is
    // dropped, so a check in progress is always counted back.
    let password = body.password;
    let shared = app.clone();
    let result = tokio::task::spawn_blocking(move || {
        let matched = Auth::verify_password(&hash, &password);
        shared.auth.lock().unwrap().finish_login(matched)
    })
    .await
    .unwrap_or(Err(Refusal::Unauthenticated));
    match result {
        Ok((cookie, csrf)) => {
            ([(header::SET_COOKIE, auth::set_cookie(&cookie))], Json(json!({"csrf": csrf}))).into_response()
        }
        Err(r) => refusal(r),
    }
}

/// A demo visitor's login, without a password. A visitor whose login is still live gets their visit back; anyone else
/// reserves their client's place (see `demo`), then gets a login and a session at the first demo, or the one asked
/// for, owned by that login, until the deadline.
async fn demo_login(app: Shared, headers: HeaderMap, body: LoginBody) -> Response {
    let Some(demo) = app.config.demo.as_ref() else { return not_in_the_demo() };
    // Checked into a local first: a guard in the condition would hold the auth lock across `demo::info`, which takes
    // the visits lock (see [`App`] for the lock order).
    let live = app.auth.lock().unwrap().check(&Method::GET, &headers, &app.config.origin);
    if let Ok((login, csrf)) = live
        && let Some(info) = crate::demo::info(&app, login)
    {
        return Json(json!({"csrf": csrf, "demo": info})).into_response();
    }
    let name = body.demo.or_else(|| demo.sites.first().map(|(name, _)| name.clone())).unwrap_or_default();
    let Some(url) = demo.start_url(&name).map(str::to_owned) else {
        return error(StatusCode::BAD_REQUEST, "pick one of the demos");
    };
    // Cloudflare's tunnel sets this at its edge, and the demo admin is reachable only through the tunnel.
    let address = headers.get("cf-connecting-ip").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
    let Some(client) = address.map(crate::demo::Client::of) else {
        return error(StatusCode::BAD_REQUEST, "no client address");
    };
    // This client's visit is open in another browser: it is closed (its browser deleted) and this one starts, in the
    // place it keeps meanwhile. A start this client may not make now is refused, leaving that visit as it is. A start
    // the page made on its own never closes another: it waits for the place.
    let taken = if body.wait { Ok(None) } else { app.demo_visits.take_over(client) };
    match taken {
        Ok(Some((old, gone))) => crate::demo::close_taken_over(&app, old, gone).await,
        Ok(None) => {
            if let Err(refused) = app.demo_visits.reserve(client, app.config.max_sessions) {
                return refused_start(refused);
            }
        }
        Err(refused) => return refused_start(refused),
    }
    let (cookie, csrf) = app.auth.lock().unwrap().open_login();
    let login = auth::key_of(&cookie);
    let id = session_id();
    let start = Start {
        id: id.clone(),
        url,
        devtools: false,
        egress: None,
        kiosk: false,
        lane: None,
        owner: app::Owner::Visitor(login),
        features: app::Features::default(),
    };
    // The visit's time counts from here, so its session is never older than the visit, whatever the start took.
    let began = std::time::Instant::now();
    match service::start(&app, start).await {
        Ok(session) => {
            crate::demo::activate(&app, client, login, session.id.clone(), name, began, demo.seconds);
            let info = crate::demo::info(&app, login);
            ([(header::SET_COOKIE, auth::set_cookie(&cookie))], Json(json!({"csrf": csrf, "demo": info})))
                .into_response()
        }
        Err((status, why)) => {
            // The place is given back once whatever the failed start left behind is confirmed gone.
            app.auth.lock().unwrap().logout(login);
            crate::demo::failed_start(&app, client, id);
            error(status, why)
        }
    }
}

/// Why a demo visit cannot start now, and when to ask again.
fn refused_start(refused: crate::demo::Refused) -> Response {
    use crate::demo::Refused;
    let (status, reason, why, wait) = match refused {
        Refused::Opening(w) => (StatusCode::CONFLICT, "opening", "the demo is opening", w),
        Refused::Full(w) => (StatusCode::CONFLICT, "full", "every demo browser is in use", w),
        Refused::Yours(w) => (StatusCode::TOO_MANY_REQUESTS, "yours", "this address already has a demo open", w),
        Refused::TooOften(w) => (StatusCode::TOO_MANY_REQUESTS, "often", "too many demos from this address", w),
    };
    (
        status,
        [(header::RETRY_AFTER, wait.to_string())],
        Json(json!({"error": why, "reason": reason, "retry_after": wait})),
    )
        .into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SwitchBody {
    demo: String,
    /// The viewer asking (its token): it must hold control.
    viewer: String,
}

async fn demo_switch(
    State(app): State<Shared>,
    method: Method,
    headers: HeaderMap,
    Json(body): Json<SwitchBody>,
) -> Response {
    let login = match authed(&app, &method, &headers) {
        Ok(key) => key,
        Err(r) => return refusal(r),
    };
    detached(switch_demo(app.clone(), login, body.demo, body.viewer)).await
}

/// A demo visitor's switch to another demo: the browser opens that demo's start page in place of its pages, in the
/// same session and visit (the deadline stays). A failure leaves the visit as it was.
async fn switch_demo(app: Shared, login: auth::LoginKey, name: String, viewer: String) -> Response {
    let Some(demo) = app.config.demo.as_ref() else { return not_in_the_demo() };
    let Some(url) = demo.start_url(&name).map(str::to_owned) else {
        return error(StatusCode::BAD_REQUEST, "pick one of the demos");
    };
    let switch = match crate::demo::begin_switch(&app, login) {
        Ok(switch) => switch,
        Err(None) => return error(StatusCode::GONE, "your turn is over"),
        Err(Some(())) => return error(StatusCode::TOO_MANY_REQUESTS, "a switch is already on its way"),
    };
    // Switches of one visit go one after another (each is a page opening, quick); the visit's end waits for them.
    let life = switch.life.clone();
    let _life = life.lock().await;
    let number = switch.number;
    let Some(session) = crate::demo::active_session(&app, login).and_then(|id| app.session(&id)) else {
        return error(StatusCode::GONE, "your turn is over");
    };
    // Only the viewer asking, and only while it holds control: another tab of the same visitor, watching, cannot
    // change the page under the one driving.
    let (lease_epoch, to_viewer) = {
        let state = session.state.lock().unwrap();
        let asking = Session::viewer_by_token(&state, login, &viewer);
        match asking.filter(|v| state.holder == Some(*v)).and_then(|v| state.viewers.get(&v)) {
            Some(entry) => (state.lease_epoch, entry.tx.clone()),
            None => return error(StatusCode::CONFLICT, "switching needs your turn at the screen"),
        }
    };
    let (opened, mut answer) = tokio::sync::oneshot::channel();
    let asked = async {
        to_viewer.send(app::ToViewer::Open { lease_epoch, url, opened }).await.map_err(|_| ())?;
        (&mut answer).await.map_err(|_| ())
    };
    match tokio::time::timeout(OPEN_DEADLINE, asked).await {
        Ok(Ok(true)) => {}
        Ok(_) => return error(StatusCode::BAD_GATEWAY, "the demo did not open; try again"),
        Err(_) => {
            // Late, not refused: if the browser still opens it, the visit follows the page it shows.
            let late = app.clone();
            tokio::spawn(async move {
                if let Ok(Ok(true)) = tokio::time::timeout(OPEN_LATE, answer).await {
                    crate::demo::set_site(&late, login, name, number);
                }
            });
            return error(StatusCode::BAD_GATEWAY, "the demo did not open in time; try again");
        }
    }
    if !crate::demo::set_site(&app, login, name, number) {
        return error(StatusCode::GONE, "your turn is over");
    }
    tracing::info!(session = %session.id, "demo switched in place");
    Json(json!({"demo": crate::demo::info(&app, login)})).into_response()
}

/// How long the browser has to open a demo's page.
const OPEN_DEADLINE: Duration = Duration::from_secs(10);
/// How long a page opening that missed the deadline is still followed.
const OPEN_LATE: Duration = Duration::from_secs(30);

async fn logout(State(app): State<Shared>, method: Method, headers: HeaderMap) -> Response {
    let key = match authed(&app, &method, &headers) {
        Ok(key) => key,
        Err(r) => return refusal(r),
    };
    app.auth.lock().unwrap().logout(key);
    app.end_login(key);
    if app.config.demo.is_some() {
        let app = app.clone();
        tokio::spawn(async move { crate::demo::finish(&app, key).await });
    }
    ([(header::SET_COOKIE, auth::clear_cookie())], Json(json!({"ok": true}))).into_response()
}

async fn me(State(app): State<Shared>, method: Method, headers: HeaderMap) -> Response {
    let checked = app.auth.lock().unwrap().check(&method, &headers, &app.config.origin);
    match checked {
        Ok((login, csrf)) => {
            let demo = app.config.demo.as_ref().map(|d| {
                let sites: Vec<&str> = d.sites.iter().map(|(name, _)| name.as_str()).collect();
                json!({"sites": sites, "seconds": d.seconds, "visit": crate::demo::info(&app, login)})
            });
            Json(json!({"csrf": csrf, "demo": demo})).into_response()
        }
        Err(r) => refusal(r),
    }
}

async fn list_sessions(State(app): State<Shared>, method: Method, headers: HeaderMap) -> Response {
    let login = match authed(&app, &method, &headers) {
        Ok(login) => login,
        Err(r) => return refusal(r),
    };
    let sessions: Vec<Value> = service::sessions_of(&app, app.principal(login)).iter().map(|s| describe(s)).collect();
    Json(json!({"sessions": sessions})).into_response()
}

/// The egresses a session may leave through: their names and where their browsers say they are, never their proxies.
async fn list_egresses(State(app): State<Shared>, method: Method, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&app, &method, &headers) {
        return refusal(r);
    }
    let shown = if app.config.demo.is_some() { &[][..] } else { &app.config.egresses[..] };
    let egresses: Vec<Value> = shown
        .iter()
        .map(|e| json!({"name": e.name, "timezone": e.locale.timezone, "languages": e.locale.languages}))
        .collect();
    Json(json!({"egresses": egresses})).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    url: String,
    #[serde(default)]
    devtools: bool,
    /// The name of a configured egress; none leaves directly.
    #[serde(default)]
    egress: Option<String>,
    /// Kiosk mode, which the controller accepts only in the bench profile.
    #[serde(default)]
    kiosk: bool,
    /// A benchmark CPU lane, which the controller accepts only in the bench profile.
    #[serde(default)]
    lane: Option<u8>,
    /// What its viewers may do; everything when left out.
    #[serde(default)]
    features: app::Features,
}

/// The network simulator's ranges.
const MAX_LATENCY_MS: u32 = 2000;
const MAX_LOSS_PERCENT: f32 = 50.0;
const MAX_BANDWIDTH_KBPS: u32 = 1_000_000;

async fn create_session(State(app): State<Shared>, _: Administrator, Json(body): Json<Create>) -> Response {
    detached(create(app, body)).await
}

async fn create(app: Shared, body: Create) -> Response {
    let (url, egress) = match service::target(&app, &body.url, body.egress.as_deref()) {
        Ok(target) => target,
        Err(why) => return error(StatusCode::BAD_REQUEST, why),
    };
    let start = Start {
        id: session_id(),
        url,
        devtools: body.devtools,
        egress,
        kiosk: body.kiosk,
        lane: body.lane,
        owner: app::Owner::Admin,
        features: body.features,
    };
    match service::start(&app, start).await {
        Ok(session) => Json(describe(&session)).into_response(),
        Err((status, why)) => error(status, why),
    }
}

/// What the demo refuses: everything that administers the installation.
fn not_in_the_demo() -> Response {
    error(StatusCode::FORBIDDEN, "not available in the demo")
}

/// Stops a session by id: never in the demo, where a visitor's session ends with their turn or their logout.
async fn stop_session(State(app): State<Shared>, _: Administrator, Path(id): Path<String>) -> Response {
    let Some(session) = app.session(&id) else { return error(StatusCode::NOT_FOUND, "no such session") };
    // Its own task: a person who disconnects mid-way cannot leave the session half stopped.
    detached(async move {
        match stop(&app, &session).await {
            Ok(()) => Json(json!({"ok": true})).into_response(),
            Err(e) => error(StatusCode::BAD_GATEWAY, e),
        }
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewerRef {
    viewer: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TakeBody {
    viewer: String,
    /// Take control only from the agent, never from another viewer (a demo visitor's automatic takeover, which must
    /// not steal it back from their other tab).
    #[serde(default)]
    if_free: bool,
}

async fn take_control(
    State(app): State<Shared>,
    method: Method,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<TakeBody>,
) -> Response {
    let login = match authed(&app, &method, &headers) {
        Ok(key) => key,
        Err(r) => return refusal(r),
    };
    detached(take(app, login, id, body)).await
}

async fn take(app: Shared, login: LoginKey, id: String, body: TakeBody) -> Response {
    let Some(session) = app.session_for(app.principal(login), &id) else {
        return error(StatusCode::NOT_FOUND, "no such session");
    };
    let _serial = session.control.lock().await;
    let viewer = {
        let state = session.state.lock().unwrap();
        if state.failed.is_some() {
            return error(StatusCode::CONFLICT, "the session was stopped after a failure");
        }
        // Only takeovers change the holder to a viewer, and they are serial: free here stays free until the grant.
        if body.if_free && state.holder.is_some() {
            return error(StatusCode::CONFLICT, "another viewer holds control");
        }
        match Session::viewer_by_token(&state, login, &body.viewer) {
            Some(viewer) => viewer,
            None => return error(StatusCode::NOT_FOUND, "no such viewer"),
        }
    };
    match session.agent.quiesce().await {
        Ok(agent::Quiescence::Idle) => {}
        Ok(agent::Quiescence::Terminated) => session.state.lock().unwrap().agent_terminated = true,
        Err(why) => {
            tracing::error!(session = %id, %why, "no proof the agent stopped; stopping the session");
            session.state.lock().unwrap().failed = Some(why.clone());
            let _ = stop(&app, &session).await;
            return error(StatusCode::CONFLICT, format!("the session was stopped after a failure: {why}"));
        }
    }
    let granted = {
        let mut state = session.state.lock().unwrap();
        // The login may have ended (a demo visitor's visit) while the agent was being stopped; checked under the
        // state lock, which a logout's stream closing also takes.
        let live = app.auth.lock().unwrap().is_live(&login);
        if live && state.viewers.contains_key(&viewer) { Some(Session::grant(&mut state, viewer)) } else { None }
    };
    let Some(epoch) = granted else {
        // Another viewer may still hold control (this takeover did not ask for a free lease).
        session.resume_unheld().await;
        return error(StatusCode::GONE, "the viewer left");
    };
    tracing::info!(session = %id, viewer, epoch, "control granted");
    Json(json!({"lease_epoch": epoch})).into_response()
}

async fn give_back(
    State(app): State<Shared>,
    method: Method,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<ViewerRef>,
) -> Response {
    let login = match authed(&app, &method, &headers) {
        Ok(key) => key,
        Err(r) => return refusal(r),
    };
    detached(give(app, login, id, body)).await
}

async fn give(app: Shared, login: LoginKey, id: String, body: ViewerRef) -> Response {
    let Some(session) = app.session_for(app.principal(login), &id) else {
        return error(StatusCode::NOT_FOUND, "no such session");
    };
    let _serial = session.control.lock().await;
    let terminated = {
        let mut state = session.state.lock().unwrap();
        let holder = Session::viewer_by_token(&state, login, &body.viewer);
        if state.holder.is_none() || state.holder != holder {
            return error(StatusCode::CONFLICT, "this viewer does not hold control");
        }
        state.redactions.boundary();
        Session::revoke(&mut state);
        std::mem::take(&mut state.agent_terminated)
    };
    if let Err(why) = session.agent.resume(terminated).await {
        session.state.lock().unwrap().failed = Some(why.clone());
        return error(StatusCode::BAD_GATEWAY, why);
    }
    tracing::info!(session = %id, "control given back to the agent");
    Json(json!({"ok": true})).into_response()
}

async fn set_settings(
    State(app): State<Shared>,
    _: Administrator,
    Path(id): Path<String>,
    Json(settings): Json<Settings>,
) -> Response {
    if !frame::BITRATE_KBPS.contains(&settings.bitrate_kbps) || !frame::MAX_FPS.contains(&settings.max_fps) {
        let (bitrate, fps) = (frame::BITRATE_KBPS, frame::MAX_FPS);
        return error(
            StatusCode::BAD_REQUEST,
            format!(
                "bitrate {}-{} kbit/s, frame-rate cap {}-{}",
                bitrate.start(),
                bitrate.end(),
                fps.start(),
                fps.end()
            ),
        );
    }
    let Some(session) = app.session(&id) else { return error(StatusCode::NOT_FOUND, "no such session") };
    let mut state = session.state.lock().unwrap();
    state.settings = settings;
    for viewer in state.viewers.values() {
        let _ = viewer.tx.try_send(app::ToViewer::Display(settings.link()));
    }
    Json(json!({"ok": true})).into_response()
}

async fn set_network(
    State(app): State<Shared>,
    _: Administrator,
    Path(id): Path<String>,
    Json(network): Json<Network>,
) -> Response {
    if network.latency_ms > MAX_LATENCY_MS
        || !(0.0..=MAX_LOSS_PERCENT).contains(&network.loss_percent)
        || network.bandwidth_kbps > MAX_BANDWIDTH_KBPS
    {
        return error(
            StatusCode::BAD_REQUEST,
            format!(
                "latency up to {MAX_LATENCY_MS} ms, loss 0-{MAX_LOSS_PERCENT} %, bandwidth up to {MAX_BANDWIDTH_KBPS} kbit/s"
            ),
        );
    }
    let Some(session) = app.session(&id) else { return error(StatusCode::NOT_FOUND, "no such session") };
    session.state.lock().unwrap().network = network;
    Json(json!({"ok": true})).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentCall {
    tool: String,
    #[serde(default)]
    arguments: Value,
}

async fn agent_call(
    State(app): State<Shared>,
    _: Administrator,
    Path(id): Path<String>,
    Json(body): Json<AgentCall>,
) -> Response {
    let Some(session) = app.session(&id) else { return error(StatusCode::NOT_FOUND, "no such session") };
    called(service::call(&app, app::Owner::Admin, &session, &body.tool, body.arguments).await)
}

/// An agent call's outcome as HTTP, for the administrator's page and the agent API (which answers a revoked token
/// as a bearer refusal instead). MCP answers in tool results of its own.
pub(crate) fn called(outcome: Called) -> Response {
    match outcome {
        Called::Done(result) => Json(result).into_response(),
        Called::Stopped => error(StatusCode::CONFLICT, "the session was stopped after a failure"),
        Called::Controlled => {
            error(StatusCode::CONFLICT, "a person holds control of this session; try again once they give it back")
        }
        Called::Failed(why) => error(StatusCode::BAD_GATEWAY, why),
        Called::Revoked => refusal(Refusal::Unauthenticated),
        Called::Busy => busy(),
    }
}

async fn stats(State(app): State<Shared>, method: Method, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let login = match authed(&app, &method, &headers) {
        Ok(login) => login,
        Err(r) => return refusal(r),
    };
    let Some(session) = app.session_for(app.principal(login), &id) else {
        return error(StatusCode::NOT_FOUND, "no such session");
    };
    let containers = session.containers(&app.controller).await;
    let viewers: Vec<Value> = session
        .state
        .lock()
        .unwrap()
        .viewers
        .iter()
        .map(|(id, v)| json!({"viewer": id, "token": v.token, "stats": v.stats}))
        .collect();
    Json(json!({"session": describe(&session), "containers": containers, "viewers": viewers})).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewToken {
    name: String,
}

/// The administrator's agent tokens (see `tokens`): never in the demo.
async fn list_tokens(State(app): State<Shared>, _: Administrator) -> Response {
    Json(json!({"tokens": app.tokens.lock().unwrap().list()})).into_response()
}

/// A new agent token, shown in this response only.
async fn create_token(State(app): State<Shared>, _: Administrator, Json(body): Json<NewToken>) -> Response {
    let created = app.tokens.lock().unwrap().create(&body.name);
    match created {
        Ok((mut listed, token)) => {
            tracing::info!(token = listed["id"].as_str().unwrap_or_default(), "agent token created");
            listed["token"] = json!(token);
            ([(header::CACHE_CONTROL, "no-store")], Json(listed)).into_response()
        }
        Err(crate::tokens::CreateError::BadName) => error(StatusCode::BAD_REQUEST, "name it in 1 to 64 characters"),
        Err(crate::tokens::CreateError::TooMany) => {
            error(StatusCode::CONFLICT, format!("at most {} tokens; revoke one first", crate::tokens::MAX_TOKENS))
        }
        Err(crate::tokens::CreateError::Unsaved(why)) => error(StatusCode::INTERNAL_SERVER_ERROR, why),
    }
}

/// Revokes a token, then stops every session started with it. The revocation is written first, so whatever happens to
/// the stops, the token is refused from now on.
async fn revoke_token(State(app): State<Shared>, _: Administrator, Path(id): Path<String>) -> Response {
    let Some(id) = crate::tokens::parse_id(&id) else { return error(StatusCode::NOT_FOUND, "no such token") };
    detached(async move {
        let revoked = app.tokens.lock().unwrap().revoke(id);
        let unsaved = match revoked {
            None => return error(StatusCode::NOT_FOUND, "no such token"),
            Some(saved) => saved.err(),
        };
        tracing::info!(token = %crate::tokens::id_text(id), "agent token revoked");
        let owner = app::Owner::Token(id);
        let owned: Vec<Arc<Session>> =
            app.sessions.lock().unwrap().values().filter(|s| s.owner == owner).cloned().collect();
        let mut failures = Vec::new();
        for session in &owned {
            if let Err(why) = stop(&app, session).await {
                failures.push(format!("{}: {why}", session.id));
            }
        }
        match (unsaved, failures.is_empty()) {
            (None, true) => Json(json!({"ok": true, "stopped": owned.len()})).into_response(),
            (Some(why), _) => error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("the token is refused until a restart, but its revocation could not be written: {why}"),
            ),
            (None, false) => error(
                StatusCode::BAD_GATEWAY,
                format!("the token is revoked; stopping its sessions failed: {}", failures.join("; ")),
            ),
        }
    })
    .await
}

#[derive(Deserialize)]
struct WatchQuery {
    viewer: String,
}

async fn watch(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<WatchQuery>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let login = match app.auth.lock().unwrap().check_upgrade(&headers, &app.config.origin) {
        Ok(key) => key,
        Err(r) => return refusal(r),
    };
    if query.viewer.len() != 32 || !query.viewer.bytes().all(|b| b.is_ascii_hexdigit()) {
        return error(StatusCode::BAD_REQUEST, "the viewer token is 32 hex characters");
    }
    let Some(session) = app.session_for(app.principal(login), &id) else {
        return error(StatusCode::NOT_FOUND, "no such session");
    };
    let (place, deadline) = match app.config.demo {
        None => (None, None),
        Some(_) => {
            let Some(deadline) = crate::demo::deadline(&app, login) else {
                return error(StatusCode::GONE, "your visit is over");
            };
            match crate::demo::ViewerPlace::take(&session) {
                Some(place) => (Some(place), Some(deadline)),
                None => return error(StatusCode::TOO_MANY_REQUESTS, "this demo is already open in another tab"),
            }
        }
    };
    upgrade
        .max_message_size(display_protocol::MAX_CLIENT_MESSAGE + 16)
        .max_frame_size(display_protocol::MAX_CLIENT_MESSAGE + 16)
        .write_buffer_size(256 * 1024)
        .max_write_buffer_size(display_protocol::MAX_SERVER_MESSAGE * 2)
        .on_upgrade(move |socket| async move {
            relay::run(app, session, login, query.viewer, socket, deadline).await;
            // The demo's viewer place is held until the relay ends.
            drop(place);
        })
}

/// A HEAD would claim the ticket without taking the download.
async fn no_head() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

/// Pieces of one download waiting for the response: a window of them, plus the end.
const PIECES: usize = (display_protocol::FILE_WINDOW / display_protocol::FILE_CHUNK as u64) as usize + 2;
/// Download responses served at once, across every session (each may hold a window in memory): one per session at the
/// most sessions, so a recording in each (itself a download) still leaves room.
const MAX_DOWNLOADS: usize = 16;

/// One of the [`MAX_DOWNLOADS`] places, given back when dropped.
struct Slot(Shared);

impl Slot {
    fn take(app: &Shared) -> Option<Slot> {
        let taken =
            app.downloads.try_update(std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst, |n| {
                (n < MAX_DOWNLOADS).then_some(n + 1)
            });
        taken.is_ok().then(|| Slot(app.clone()))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.downloads.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

async fn download(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path((id, ticket)): Path<(String, String)>,
) -> Response {
    let login = match authed(&app, &Method::GET, &headers) {
        Ok(key) => key,
        Err(r) => return refusal(r),
    };
    // The admin page's own navigation; another site never carries the Strict cookie, and is refused here as well.
    if headers.get("sec-fetch-site").is_some_and(|v| v.as_bytes() != b"same-origin") {
        return refusal(Refusal::Forbidden);
    }
    use base64::Engine;
    let ticket: Option<[u8; display_protocol::FILE_TICKET]> =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&ticket).ok().and_then(|t| t.try_into().ok());
    let Some(ticket) = ticket else { return error(StatusCode::NOT_FOUND, "no such download") };
    let Some(session) = app.session_for(app.principal(login), &id) else {
        return error(StatusCode::NOT_FOUND, "no such download");
    };
    if !session.features.downloads {
        return error(StatusCode::NOT_FOUND, "no such download");
    }
    // Checked before the ticket is spent, so a refused fetch can be tried again.
    let Some(slot) = Slot::take(&app) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "too many downloads at once; try again when one ends");
    };
    let (ticket, to_viewer, wake) = {
        let mut state = session.state.lock().unwrap();
        // Single use, by its own login: taken out whatever happens next.
        let t = match state.tickets.entry(ticket) {
            std::collections::hash_map::Entry::Occupied(t) if t.get().login == login => t.remove(),
            _ => return error(StatusCode::NOT_FOUND, "no such download"),
        };
        let viewer = state.viewers.get(&t.viewer).filter(|_| state.holds(t.viewer, t.lease_epoch));
        let Some(viewer) = viewer else { return error(StatusCode::GONE, "control moved; the download ended") };
        (t, viewer.tx.clone(), viewer.wake.clone())
    };
    let (pieces, rx) = tokio::sync::mpsc::channel(PIECES);
    let consumed = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let delivered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let diversion = app::Diversion {
        file: ticket.file,
        lease_epoch: ticket.lease_epoch,
        size: ticket.size,
        pieces,
        consumed: consumed.clone(),
        dropped: dropped.clone(),
        delivered: delivered.clone(),
    };
    let (taken, answer) = tokio::sync::oneshot::channel();
    if to_viewer.send(app::ToViewer::Download(diversion, taken)).await.is_err()
        || !matches!(tokio::time::timeout(Duration::from_secs(5), answer).await, Ok(Ok(true)))
    {
        return error(StatusCode::GONE, "the download is no longer available");
    }
    let body = Download {
        _slot: slot,
        app: app.clone(),
        session,
        login,
        viewer: ticket.viewer,
        lease_epoch: ticket.lease_epoch,
        rx,
        consumed,
        dropped,
        delivered,
        wake,
        size: ticket.size,
        sent: 0,
        held: None,
        complete: false,
    };
    let mut response = axum::body::Body::from_stream(body).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // The person's browser shows progress and time left against it. Its last piece is held back until the end is
    // confirmed, so reaching this length always means the whole, checked download.
    if let Some(size) = ticket.size.filter(|&size| size > 0) {
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from(size));
    }
    if let Ok(value) = HeaderValue::from_str(&content_disposition(&ticket.name)) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

/// `attachment`, with an ASCII fallback name and the exact name in RFC 8187 form.
fn content_disposition(name: &str) -> String {
    let fallback: String =
        name.chars().map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' { c } else { '_' }).collect();
    let mut exact = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b) {
            exact.push(b as char);
        } else {
            exact.push_str(&format!("%{b:02X}"));
        }
    }
    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{exact}")
}

/// The response body of one download: what the relay received, checked before every piece against the lease and the
/// login, so a lease change or a logout ends it at once. Ending early is an error, never a clean end, so the person's
/// browser never shows a cut download as complete.
struct Download {
    _slot: Slot,
    app: Shared,
    session: Arc<Session>,
    login: LoginKey,
    viewer: u64,
    lease_epoch: u32,
    rx: tokio::sync::mpsc::Receiver<app::Piece>,
    consumed: Arc<std::sync::atomic::AtomicU64>,
    dropped: Arc<std::sync::atomic::AtomicBool>,
    delivered: Arc<std::sync::atomic::AtomicBool>,
    wake: Arc<tokio::sync::Notify>,
    /// The offered size, the bytes yielded so far, and the last piece, held until the end is confirmed.
    size: Option<u64>,
    sent: u64,
    held: Option<axum::body::Bytes>,
    /// The end was confirmed (only the held piece may still go).
    complete: bool,
}

impl futures_util::Stream for Download {
    type Item = Result<axum::body::Bytes, std::io::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use std::task::Poll;
        let ended = || Poll::Ready(Some(Err(std::io::Error::other("the download ended before it was complete"))));
        if self.complete {
            return Poll::Ready(None);
        }
        let still = self.session.state.lock().unwrap().holds(self.viewer, self.lease_epoch);
        if !still || !self.app.auth.lock().unwrap().is_live(&self.login) {
            return ended();
        }
        loop {
            match self.rx.poll_recv(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(app::Piece::Data(bytes))) => {
                    self.consumed.fetch_add(bytes.len() as u64, std::sync::atomic::Ordering::Release);
                    self.wake.notify_one();
                    self.sent += bytes.len() as u64;
                    if self.size.is_some_and(|size| self.sent >= size) {
                        // The last piece waits for the confirmed end.
                        self.held = Some(bytes);
                        continue;
                    }
                    return Poll::Ready(Some(Ok(bytes)));
                }
                Poll::Ready(Some(app::Piece::End)) => {
                    self.complete = true;
                    // Only now does the person hear the download ended (see the relay's `delivering`).
                    self.delivered.store(true, std::sync::atomic::Ordering::Release);
                    self.wake.notify_one();
                    return Poll::Ready(self.held.take().map(Ok));
                }
                Poll::Ready(Some(app::Piece::Fail) | None) => return ended(),
            }
        }
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        self.dropped.store(true, std::sync::atomic::Ordering::Release);
        self.wake.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    use super::*;

    /// Every route that administers the installation, with a body it would accept.
    const ADMINISTRATOR_ROUTES: [(&str, &str, &str); 8] = [
        ("POST", "/api/sessions", r#"{"url": "https://a/"}"#),
        ("DELETE", "/api/sessions/abc", ""),
        ("PUT", "/api/sessions/abc/settings", r#"{"bitrate_kbps": 2500, "max_fps": 60}"#),
        ("PUT", "/api/sessions/abc/network", r#"{"latency_ms": 0, "loss_percent": 0, "bandwidth_kbps": 0}"#),
        ("POST", "/api/sessions/abc/agent", r#"{"tool": "list_windows"}"#),
        ("GET", "/api/tokens", ""),
        ("POST", "/api/tokens", r#"{"name": "a"}"#),
        ("DELETE", "/api/tokens/abc", ""),
    ];

    async fn send(app: &Shared, (method, path, body): (&str, &str, &str), login: Option<(&str, &str)>) -> StatusCode {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::ORIGIN, &app.config.origin)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some((cookie, csrf)) = login {
            request =
                request.header(header::COOKIE, format!("{}={cookie}", auth::COOKIE)).header(auth::CSRF_HEADER, csrf);
        }
        let response = router(app.clone()).oneshot(request.body(Body::from(body.to_owned())).unwrap()).await.unwrap();
        response.status()
    }

    #[tokio::test]
    async fn administrator_routes_need_a_login_and_refuse_every_demo_visitor() {
        let demo = app::test_app(true);
        let (cookie, csrf) = demo.auth.lock().unwrap().open_login();
        for route in ADMINISTRATOR_ROUTES {
            assert_eq!(send(&demo, route, None).await, StatusCode::UNAUTHORIZED, "{route:?}");
            assert_eq!(send(&demo, route, Some((&cookie, &csrf))).await, StatusCode::FORBIDDEN, "{route:?}");
        }
        // Outside the demo, the same login reaches them: no such session or token here, or a body it took.
        let app = app::test_app(false);
        let (cookie, csrf) = app.auth.lock().unwrap().open_login();
        for route in ADMINISTRATOR_ROUTES {
            assert_eq!(send(&app, route, None).await, StatusCode::UNAUTHORIZED, "{route:?}");
            let status = send(&app, route, Some((&cookie, &csrf))).await;
            assert!(![StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN].contains(&status), "{route:?}: {status}");
        }
    }
}
