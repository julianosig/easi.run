//! The admin application's HTTP API and its WebSocket viewer endpoint. Every state change a login makes needs that
//! live login, the exact origin, and its CSRF token (see `auth`); setting the password, logging in and its second step
//! need the exact origin alone, and agents' routes their token. A viewer upgrade needs the exact origin and a live
//! login.
//! A download is fetched with the single-use ticket its holder was given, by the same login, while that viewer still
//! holds control at the ticket's epoch; the response is checked again before every piece.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::rejection::JsonRejection;
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
use crate::auth::{Auth, LoginKey, Opened, Refusal};
use crate::service::{Called, Start, describe, session_id, stop};
use crate::{agent, app, auth, keys, relay, service};

pub type Shared = Arc<App>;
/// Every route, the static admin page, and the security headers every response carries.
pub fn router(app: Shared) -> Router {
    let statics = ServeDir::new(app.config.static_dir.clone());
    let static_path = app.config.static_path.clone();
    let sdk_dir = app.config.sdk_dir.clone();
    // Only the pages the demo names may frame it; nothing frames any other installation.
    let ancestors =
        if app.config.frame_ancestors.is_empty() { "'none'".to_owned() } else { app.config.frame_ancestors.join(" ") };
    // The page may also reach the direct path's address (WebTransport is governed by connect-src).
    let connect = app.udp.as_ref().map_or_else(|| "'self'".to_owned(), |udp| format!("'self' {}", udp.origin()));
    let policy = format!(
        "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' blob: data:; connect-src {connect}; \
         frame-ancestors {ancestors}; base-uri 'none'; form-action 'self'"
    );
    let router = Router::new()
        .route("/api/status", get(status))
        .route("/api/enroll", post(enroll))
        .route("/api/login", post(login))
        .route("/api/login/code", post(login_code))
        .route("/api/login/passkey", post(begin_passkey_sign_in))
        .route("/api/login/passkey/finish", post(finish_passkey_sign_in))
        .route("/api/demo/switch", post(demo_switch))
        .route("/api/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/egresses", get(list_egresses).post(create_egress))
        .route("/api/egresses/options", get(egress_options))
        .route("/api/egresses/check", post(check_draft))
        .route("/api/egresses/{id}", axum::routing::patch(update_egress).delete(delete_egress))
        .route("/api/egresses/{id}/check", post(check_egress))
        .route("/api/sessions", get(list_sessions).post(create_session))
        .route("/api/sessions/{id}", axum::routing::delete(stop_session))
        .route("/api/sessions/{id}/control", post(take_control))
        .route("/api/sessions/{id}/release", post(give_back))
        .route("/api/sessions/{id}/settings", put(set_settings))
        .route("/api/sessions/{id}/network", put(set_network))
        .route("/api/sessions/{id}/agent", post(agent_call).layer(DefaultBodyLimit::max(crate::tools::BODY_LIMIT)))
        .route("/api/sessions/{id}/stats", get(stats))
        .route("/api/sessions/{id}/files/{ticket}", get(download).head(no_head))
        .route("/ws/sessions/{id}", get(watch))
        .route("/api/tokens", get(list_tokens).post(create_token))
        .route("/api/tokens/{id}", axum::routing::delete(revoke_token))
        .route("/api/profiles", get(list_profiles).post(create_profile))
        .route("/api/profiles/unlock", post(unlock_profiles))
        .route("/api/profiles/{id}", put(update_profile).delete(delete_profile))
        .route("/api/security", get(security))
        .route("/api/security/app", post(begin_app))
        .route("/api/security/app/confirm", post(confirm_app))
        .route("/api/security/app/remove", post(remove_app))
        .route("/api/security/recovery", post(new_recovery_codes))
        .route("/api/security/passkeys", post(begin_passkey))
        .route("/api/security/passkeys/finish", post(finish_passkey))
        .route("/api/security/passkeys/{id}/remove", post(remove_passkey))
        .merge(crate::agent_api::routes())
        .route("/mcp", post(crate::mcp::endpoint).layer(DefaultBodyLimit::max(crate::tools::BODY_LIMIT)))
        .nest_service("/sdk", ServeDir::new(sdk_dir));
    // The web client at the root, or under its own path: there `/demo` itself sends the browser on to `/demo/`, where
    // the page's relative links resolve.
    let mut client = Router::new().fallback_service(statics);
    if static_path != "/" {
        let at = static_path.clone();
        client = client.layer(axum::middleware::from_fn(move |request, next| to_slash(at.clone(), request, next)));
    }
    // Outside the demo, which its tunnel reaches at its one address and which has no password.
    if app.config.demo.is_none() {
        let origin = app.config.origin.clone();
        client = client.layer(axum::middleware::from_fn(move |request, next| to_origin(origin.clone(), request, next)));
    }
    let router =
        if static_path == "/" { router.fallback_service(client) } else { router.nest_service(&static_path, client) };
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
        // The page and its script are revalidated on every load (an unchanged file costs a 304), so a page loaded after
        // an update speaks its protocol; a tab already open keeps the script it loaded until it reloads.
        .layer(SetResponseHeaderLayer::if_not_present(header::CACHE_CONTROL, HeaderValue::from_static("no-cache")))
        .with_state(app)
}

/// A page of the web client asked for at another address than the admin's own (`https://127.0.0.1:9443` for
/// `https://localhost:9443`, another tunnel port, the server's IP): sent on to the same path and query at `origin`,
/// where its login and every change belong. A browser keeps the address's fragment across a redirect, so a watch link
/// opened at the other address still names its session. The target is always `origin` (`ADMIN_ORIGIN`), with only the
/// request's path after it, never a host the request names. A request that names no host passes, as nothing says it
/// is elsewhere.
async fn to_origin(origin: String, request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let named = request
        .uri()
        .authority()
        .map(|a| a.as_str().to_owned())
        .or_else(|| request.headers().get(header::HOST).and_then(|h| h.to_str().ok()).map(str::to_owned));
    let safe = matches!(*request.method(), Method::GET | Method::HEAD);
    // As a browser would serialize it (host lowercased, no default port), so the admin's own address in another
    // spelling is not sent on to itself.
    let here = |host: &str| url::Url::parse(&format!("https://{host}")).ok().map(|u| u.origin().ascii_serialization());
    if !safe || named.is_none_or(|host| here(&host).as_deref() == Some(origin.as_str())) {
        return next.run(request).await;
    }
    let uri = request.extensions().get::<axum::extract::OriginalUri>().map_or(request.uri(), |o| &o.0);
    let target = format!("{origin}{}", uri.path_and_query().map_or("/", |p| p.as_str()));
    match HeaderValue::from_str(&target) {
        Ok(location) => (StatusCode::TEMPORARY_REDIRECT, [(header::LOCATION, location)]).into_response(),
        Err(_) => error(StatusCode::BAD_REQUEST, "bad address"),
    }
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

/// A refusal as the administrator's page takes it, so a handler may answer one with `?` (and any other answer, a
/// refusal of its own included, as `Ok`).
impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        match self {
            Refusal::Unauthenticated => error(StatusCode::UNAUTHORIZED, "log in first"),
            Refusal::Forbidden => error(StatusCode::FORBIDDEN, "forbidden"),
            // Its own refusal, naming the admin's address, so the page can send the person there rather than say that
            // their password is wrong.
            Refusal::WrongOrigin(origin) => {
                let why = format!("easi.run is set up for {origin}: open it there");
                (StatusCode::FORBIDDEN, Json(json!({"error": why, "reason": "wrong-origin", "origin": origin})))
                    .into_response()
            }
            Refusal::BadRequest(why) => error(StatusCode::BAD_REQUEST, why),
            Refusal::TooManyAttempts => error(StatusCode::TOO_MANY_REQUESTS, "too many attempts; wait"),
            Refusal::Again => {
                let why = "that took too long; enter your password again";
                (StatusCode::UNAUTHORIZED, Json(json!({"error": why, "reason": "again"}))).into_response()
            }
            // Not a 401: the login is fine, and the page must not take it for one that lapsed.
            Refusal::WrongPassword => {
                let why = "that is not your current password";
                (StatusCode::FORBIDDEN, Json(json!({"error": why, "reason": "password"}))).into_response()
            }
            Refusal::Failed(why) => error(StatusCode::INTERNAL_SERVER_ERROR, why),
        }
    }
}

/// A refusal or failure as JSON, `{"error": why}`: the one shape the administrator's page and the agent API share.
pub(crate) fn error(status: StatusCode, why: impl ToString) -> Response {
    (status, Json(json!({"error": why.to_string()}))).into_response()
}

/// An agent call refused because the session's agent is busy (see `agent::MAX_CALLS`): 503, try again shortly.
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

/// A live login (with the exact origin and CSRF token on a state change), checked before the request's body is read,
/// and its CSRF token. The cookie routes open to demo visitors take it and reach sessions through `App::session_for`.
struct Login(LoginKey, String);

impl axum::extract::FromRequestParts<Shared> for Login {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut axum::http::request::Parts, app: &Shared) -> Result<Self, Response> {
        let checked = app.auth.lock().unwrap().check(&parts.method, &parts.headers, &app.config.origin);
        checked.map(|(key, csrf)| Login(key, csrf)).map_err(Refusal::into_response)
    }
}

/// The administrator: a [`Login`] outside the demo. Every route that administers the installation takes it, so none
/// can leave out the demo's refusal: each one reaches every session, which a demo visitor must not.
struct Administrator(LoginKey);

impl axum::extract::FromRequestParts<Shared> for Administrator {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut axum::http::request::Parts, app: &Shared) -> Result<Self, Response> {
        let Login(login, _) = Login::from_request_parts(parts, app).await?;
        if app.config.demo.is_some() {
            return Err(not_in_the_demo());
        }
        Ok(Administrator(login))
    }
}

fn exact_origin(app: &App, headers: &HeaderMap) -> Result<(), Refusal> {
    if auth::origin_matches(headers, &app.config.origin) {
        Ok(())
    } else {
        Err(Refusal::WrongOrigin(app.config.origin.clone()))
    }
}

async fn status(State(app): State<Shared>) -> Response {
    let demo = app.config.demo.as_ref();
    let demo =
        demo.map(|d| json!({"sites": d.site_names(), "seconds": d.seconds, "embedders": app.config.frame_ancestors}));
    let auth = app.auth.lock().unwrap();
    // Whether the login screen offers a passkey: one is added, and this address can have them.
    Json(json!({"enrolled": auth.enrolled(), "passkey": auth.passkey_sign_in(), "demo": demo})).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Enroll {
    password: String,
}

/// The first password, while none is set (see `Auth::check_enrollment`): from the exact origin, like every change.
async fn enroll(State(app): State<Shared>, headers: HeaderMap, Json(body): Json<Enroll>) -> Result<Response, Refusal> {
    exact_origin(&app, &headers)?;
    app.auth.lock().unwrap().check_enrollment(&body.password)?;
    let password = body.password;
    let salt = app.auth.lock().unwrap().salt();
    // The hash, and the profiles' KEK the password derives (see `keys`).
    let hashed = tokio::task::spawn_blocking(move || {
        Auth::hash_password(&password).map(|hash| (hash, salt.map(|salt| keys::derive(&password, &salt))))
    });
    let (hash, unlock) = match hashed.await {
        Ok(hashed) => hashed?,
        Err(e) => return Ok(error(StatusCode::INTERNAL_SERVER_ERROR, e)),
    };
    // Setting the password logs in too, exactly as a login does.
    let (cookie, csrf) = app.auth.lock().unwrap().complete_enrollment(hash, unlock)?;
    Ok(logged_in(&cookie, csrf))
}

/// A login's answer: its cookie, and the CSRF token every state change it makes must echo.
fn logged_in(cookie: &str, csrf: String) -> Response {
    ([(header::SET_COOKIE, auth::set_cookie(cookie))], Json(json!({"csrf": csrf}))).into_response()
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

async fn login(
    State(app): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> Result<Response, Refusal> {
    exact_origin(&app, &headers)?;
    if app.config.demo.is_some() {
        return Ok(detached(demo_login(app.clone(), headers, body)).await);
    }
    let hash = app.auth.lock().unwrap().begin_login()?;
    // The check and its bookkeeping run together on a blocking thread, which finishes even if this request is
    // dropped, so a check in progress is always counted back. A right password also derives the profiles' KEK.
    let password = body.password;
    let salt = app.auth.lock().unwrap().salt();
    let shared = app.clone();
    let result = tokio::task::spawn_blocking(move || {
        let matched = Auth::verify_password(&hash, &password);
        let unlock = matched.then_some(salt).flatten().map(|salt| keys::derive(&password, &salt));
        shared.auth.lock().unwrap().finish_login(matched, unlock)
    })
    .await
    .unwrap_or(Err(Refusal::Unauthenticated));
    Ok(match result? {
        Opened::Login(cookie, csrf) => logged_in(&cookie, csrf),
        // No cookie yet: the pending step's token alone, which only answering it turns into a login.
        Opened::SecondStep(step, methods) => {
            secret_answer(json!({"second_step": {"step": step, "app": methods.app, "passkey": methods.passkey}}))
        }
    })
}

/// Begins a passkey sign-in, which needs no password: a passkey is two factors on its own.
async fn begin_passkey_sign_in(State(app): State<Shared>, headers: HeaderMap) -> Result<Response, Refusal> {
    exact_origin(&app, &headers)?;
    if app.config.demo.is_some() {
        return Ok(not_in_the_demo());
    }
    let (ceremony, options) = app.auth.lock().unwrap().begin_passkey_sign_in()?;
    Ok(Json(json!({"ceremony": ceremony, "options": options})).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasskeySignIn {
    ceremony: String,
    credential: webauthn_rs::prelude::PublicKeyCredential,
}

async fn finish_passkey_sign_in(
    State(app): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<PasskeySignIn>,
) -> Result<Response, Refusal> {
    exact_origin(&app, &headers)?;
    if app.config.demo.is_some() {
        return Ok(not_in_the_demo());
    }
    let (cookie, csrf) = app.auth.lock().unwrap().finish_passkey_sign_in(&body.ceremony, &body.credential)?;
    Ok(logged_in(&cookie, csrf))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeBody {
    step: String,
    code: String,
}

/// The second step after the password: a code from the authenticator app, or a recovery code, for the pending step
/// the password opened.
async fn login_code(
    State(app): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<CodeBody>,
) -> Result<Response, Refusal> {
    exact_origin(&app, &headers)?;
    if app.config.demo.is_some() {
        return Ok(not_in_the_demo());
    }
    let (cookie, csrf) = app.auth.lock().unwrap().sign_in_with_code(&body.step, &body.code)?;
    Ok(logged_in(&cookie, csrf))
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
        profile: None,
        size: None,
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
        Err(refused) => {
            // The place is given back once whatever the failed start left behind is confirmed gone.
            app.auth.lock().unwrap().logout(login);
            crate::demo::failed_start(&app, client, id);
            refused.response()
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

async fn demo_switch(State(app): State<Shared>, Login(login, _): Login, Json(body): Json<SwitchBody>) -> Response {
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

async fn logout(State(app): State<Shared>, Login(key, _): Login) -> Response {
    app.auth.lock().unwrap().logout(key);
    app.end_login(key, crate::app::ENDED);
    if app.config.demo.is_some() {
        let app = app.clone();
        tokio::spawn(async move { crate::demo::end(&app, key, crate::app::ENDED).await });
    }
    ([(header::SET_COOKIE, auth::clear_cookie())], Json(json!({"ok": true}))).into_response()
}

async fn me(State(app): State<Shared>, Login(login, csrf): Login) -> Response {
    let demo = app.config.demo.as_ref();
    let demo =
        demo.map(|d| json!({"sites": d.site_names(), "seconds": d.seconds, "visit": crate::demo::info(&app, login)}));
    Json(json!({"csrf": csrf, "demo": demo})).into_response()
}

async fn list_sessions(State(app): State<Shared>, Login(login, _): Login) -> Response {
    let sessions: Vec<Value> = service::sessions_of(&app, app.principal(login)).iter().map(|s| describe(s)).collect();
    Json(json!({"sessions": sessions})).into_response()
}

/// An answer no cache keeps: an egress's, or one that holds a secret (see [`secret_answer`]).
fn unstored(status: StatusCode, body: Value) -> Response {
    (status, [(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

fn egress_refused(refused: crate::egresses::Refused) -> Response {
    let (status, body) = refused.answer();
    unstored(status, body)
}

/// A check that did not run: 503 with `Retry-After` while other checks are under way, else 502.
fn unchecked(unchecked: crate::egresses::Unchecked) -> Response {
    let (status, why) = unchecked.answer();
    let mut response = unstored(status, json!({"error": why}));
    if status == StatusCode::SERVICE_UNAVAILABLE {
        response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("2"));
    }
    response
}

/// A check that failed, refusing a save: 422, `{"error", "reason", "detail"}`.
fn failed_check(reason: &str, detail: &str) -> Response {
    let why = format!("the proxy did not pass its check: {detail}");
    unstored(StatusCode::UNPROCESSABLE_ENTITY, json!({"error": why, "reason": reason, "detail": detail}))
}

/// The egresses (see `egresses`), never a credential: never in the demo.
async fn list_egresses(State(app): State<Shared>, _: Administrator) -> Response {
    let egresses = app.egresses.lock().unwrap().list();
    unstored(StatusCode::OK, json!({"egresses": egresses, "max": crate::egresses::MAX_EGRESSES}))
}

/// The time zones and languages the page offers first when the administrator overrides an egress's.
async fn egress_options(_: Administrator) -> Response {
    let languages: Vec<Value> =
        locale::COMMON_LANGUAGES.iter().map(|(tag, name)| json!({"tag": tag, "name": name})).collect();
    Json(json!({"timezones": locale::COMMON_TIMEZONES, "languages": languages})).into_response()
}

/// A field that, when present, must be a value: `null` is refused rather than taken as absent.
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewEgress {
    name: String,
    /// The full proxy URL, credentials included.
    proxy: String,
    /// Where its browsers say they are; what its country suggests when left out.
    #[serde(default, deserialize_with = "present")]
    timezone: Option<String>,
    #[serde(default, deserialize_with = "present")]
    languages: Option<Vec<String>>,
}

/// Checks `upstream` as a session would use it, its placeholder filled with a fresh session-like id.
async fn checked(
    app: &App,
    upstream: &egress::Upstream,
) -> Result<crate::egresses::Checked, crate::egresses::Unchecked> {
    // Never fails: every proxy taken was checked with a session's id in it.
    let filled = upstream
        .for_session(&service::session_id())
        .map_err(|why| crate::egresses::Unchecked::Failed(why.to_owned()))?;
    crate::egresses::check(&app.controller, &filled).await
}

/// Adds an egress: its proxy is checked first, and the egress saved only if the check passed, with the country it
/// found and, unless given, the time zone and languages that country suggests. The listing, with the check's latency.
async fn create_egress(State(app): State<Shared>, _: Administrator, Json(body): Json<NewEgress>) -> Response {
    detached(async move {
        let admitted = app.egresses.lock().unwrap().admits(&body.name);
        let upstream = match admitted.and_then(|_| crate::egresses::proxy(&body.proxy)) {
            Ok(upstream) => upstream,
            Err(refused) => return egress_refused(refused),
        };
        let (latency_ms, country) = match checked(&app, &upstream).await {
            Ok(crate::egresses::Checked::Passed { latency_ms, country, .. }) => (latency_ms, country),
            Ok(crate::egresses::Checked::Failed { reason, detail }) => return failed_check(&reason, &detail),
            Err(why) => return unchecked(why),
        };
        let locale = match crate::egresses::locale_for(body.timezone, body.languages, &country) {
            Ok(locale) => locale,
            Err(refused) => return egress_refused(refused),
        };
        let created = app.egresses.lock().unwrap().create(&body.name, upstream, locale, country);
        match created {
            Ok(mut listed) => {
                tracing::info!(egress = listed["id"].as_str().unwrap_or_default(), "egress added");
                listed["latency_ms"] = json!(latency_ms);
                unstored(StatusCode::OK, listed)
            }
            Err(refused) => egress_refused(refused),
        }
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EgressChange {
    #[serde(default, deserialize_with = "present")]
    name: Option<String>,
    /// A new proxy URL, which replaces the address and credentials whole once it passed its check.
    #[serde(default, deserialize_with = "present")]
    proxy: Option<String>,
    #[serde(default, deserialize_with = "present")]
    timezone: Option<String>,
    #[serde(default, deserialize_with = "present")]
    languages: Option<Vec<String>>,
}

/// Changes an egress. A new proxy is checked first and replaces the old one only if it passed, with the country it
/// found and, unless given, the time zone and languages that country suggests; sessions already started keep theirs.
async fn update_egress(
    State(app): State<Shared>,
    _: Administrator,
    Path(id): Path<String>,
    Json(body): Json<EgressChange>,
) -> Response {
    detached(async move {
        if app.egresses.lock().unwrap().get(&id).is_none() {
            return egress_refused(crate::egresses::Refused::NoSuch);
        }
        let (timezone, languages) = (body.timezone, body.languages);
        let mut change = crate::egresses::Change { name: body.name, ..Default::default() };
        let mut latency = None;
        if let Some(text) = body.proxy {
            let upstream = match crate::egresses::proxy(&text) {
                Ok(upstream) => upstream,
                Err(refused) => return egress_refused(refused),
            };
            let (latency_ms, country) = match checked(&app, &upstream).await {
                Ok(crate::egresses::Checked::Passed { latency_ms, country, .. }) => (latency_ms, country),
                Ok(crate::egresses::Checked::Failed { reason, detail }) => return failed_check(&reason, &detail),
                Err(why) => return unchecked(why),
            };
            // What is not given follows the new country.
            let locale = match crate::egresses::locale_for(timezone, languages, &country) {
                Ok(locale) => locale,
                Err(refused) => return egress_refused(refused),
            };
            (change.timezone, change.languages) = (Some(locale.timezone), Some(locale.languages));
            change.upstream = Some((upstream, country));
            latency = Some(latency_ms);
        } else {
            // Merged into the egress as it is when the change applies, under its lock.
            (change.timezone, change.languages) = (timezone, languages);
        }
        let updated = app.egresses.lock().unwrap().update(&id, change);
        match updated {
            Ok(mut listed) => {
                if let Some(latency_ms) = latency {
                    listed["latency_ms"] = json!(latency_ms);
                }
                unstored(StatusCode::OK, listed)
            }
            Err(refused) => egress_refused(refused),
        }
    })
    .await
}

/// Deletes an egress, unless a session holds it (409, naming the sessions).
async fn delete_egress(State(app): State<Shared>, _: Administrator, Path(id): Path<String>) -> Response {
    let deleted = app.egresses.lock().unwrap().delete(&id);
    match deleted {
        Ok(()) => {
            tracing::info!(egress = %id, "egress deleted");
            unstored(StatusCode::OK, json!({"ok": true}))
        }
        Err(refused) => egress_refused(refused),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Draft {
    proxy: String,
}

/// Checks a proxy that is not saved: what the check found, which saves nothing.
async fn check_draft(State(app): State<Shared>, _: Administrator, Json(body): Json<Draft>) -> Response {
    let upstream = match crate::egresses::proxy(&body.proxy) {
        Ok(upstream) => upstream,
        Err(refused) => return egress_refused(refused),
    };
    detached(async move {
        match checked(&app, &upstream).await {
            Ok(found) => unstored(StatusCode::OK, found.answer()),
            Err(why) => unchecked(why),
        }
    })
    .await
}

/// Checks a saved egress as a session would use it: what the check found, which a session's start may reuse (see
/// `egresses`) and whose country the egress keeps.
async fn check_egress(State(app): State<Shared>, _: Administrator, Path(id): Path<String>) -> Response {
    let Some(egress) = app.egresses.lock().unwrap().get(&id) else {
        return egress_refused(crate::egresses::Refused::NoSuch);
    };
    detached(async move {
        let began = std::time::Instant::now();
        match checked(&app, &egress.upstream).await {
            Ok(found) => {
                app.egresses.lock().unwrap().record(&egress, began, &found);
                unstored(StatusCode::OK, found.answer())
            }
            Err(why) => unchecked(why),
        }
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    url: String,
    #[serde(default)]
    devtools: bool,
    /// The id of an egress (see `egresses`); none leaves directly.
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
    /// The id of a saved profile to start from (see `profiles`).
    #[serde(default)]
    profile: Option<String>,
    /// Its screen's width and height, both or neither: the window that will watch it asks for its own.
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
}

/// The network simulator's ranges.
const MAX_LATENCY_MS: u32 = 2000;
const MAX_LOSS_PERCENT: f32 = 50.0;
const MAX_BANDWIDTH_KBPS: u32 = 1_000_000;

async fn create_session(State(app): State<Shared>, _: Administrator, Json(body): Json<Create>) -> Response {
    detached(create(app, body)).await
}

async fn create(app: Shared, body: Create) -> Response {
    let url = match service::target(&body.url) {
        Ok(url) => url,
        Err(why) => return error(StatusCode::BAD_REQUEST, why),
    };
    let size = match (body.width, body.height) {
        (None, None) => None,
        (Some(w), Some(h)) if display_protocol::frame::output_size_ok(w, h) => Some((w, h)),
        _ => {
            let rule = display_protocol::frame::OUTPUT_SIZE_RULE;
            return error(StatusCode::BAD_REQUEST, format!("width and height go together, {rule}"));
        }
    };
    let egress = body.egress.map(service::EgressChoice::Id);
    let start = Start {
        id: session_id(),
        url,
        devtools: body.devtools,
        egress,
        kiosk: body.kiosk,
        lane: body.lane,
        owner: app::Owner::Admin,
        features: body.features,
        profile: body.profile,
        size,
    };
    match service::start(&app, start).await {
        Ok(session) => Json(describe(&session)).into_response(),
        Err(refused) => refused.response(),
    }
}

/// A session that is not there, or not the caller's.
pub(crate) fn no_session() -> Response {
    error(StatusCode::NOT_FOUND, "no such session")
}

/// What the demo refuses: everything that administers the installation.
fn not_in_the_demo() -> Response {
    error(StatusCode::FORBIDDEN, "not available in the demo")
}

/// Stops a session by id: never in the demo, where a visitor's session ends with their turn or their logout.
async fn stop_session(State(app): State<Shared>, _: Administrator, Path(id): Path<String>) -> Response {
    let Some(session) = app.session(&id) else { return no_session() };
    stopped(app, session).await
}

/// Stops `session` as its own task (see [`detached`]): `{"ok": true}`, or 502 with why it could not be stopped.
pub(crate) async fn stopped(app: Shared, session: Arc<Session>) -> Response {
    detached(async move {
        match stop(&app, &session).await {
            Ok(()) => Json(json!({"ok": true})).into_response(),
            Err(why) => error(StatusCode::BAD_GATEWAY, why),
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
    Login(login, _): Login,
    Path(id): Path<String>,
    Json(body): Json<TakeBody>,
) -> Response {
    detached(take(app, login, id, body)).await
}

async fn take(app: Shared, login: LoginKey, id: String, body: TakeBody) -> Response {
    let Some(session) = app.session_for(app.principal(login), &id) else { return no_session() };
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
        // A stop marks the session ended under this lock before it revokes the lease: no grant comes after it.
        let open = live && state.viewers.contains_key(&viewer) && !*session.ended.borrow();
        if open { Some(Session::grant(&mut state, viewer)) } else { None }
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
    Login(login, _): Login,
    Path(id): Path<String>,
    Json(body): Json<ViewerRef>,
) -> Response {
    detached(give(app, login, id, body)).await
}

async fn give(app: Shared, login: LoginKey, id: String, body: ViewerRef) -> Response {
    let Some(session) = app.session_for(app.principal(login), &id) else { return no_session() };
    let _serial = session.control.lock().await;
    let terminated = {
        let mut state = session.state.lock().unwrap();
        let holder = Session::viewer_by_token(&state, login, &body.viewer);
        if state.holder.is_none() || state.holder != holder {
            return error(StatusCode::CONFLICT, "this viewer does not hold control");
        }
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
    let Some(session) = app.session(&id) else { return no_session() };
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
    let Some(session) = app.session(&id) else { return no_session() };
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

/// One tool call of the administrator's: the agent API's body limit (see `tools::BODY_LIMIT`), and a body it refuses
/// answered as JSON, as every other answer of this route is.
async fn agent_call(
    State(app): State<Shared>,
    _: Administrator,
    Path(id): Path<String>,
    body: Result<Json<AgentCall>, JsonRejection>,
) -> Response {
    let body = match body {
        Ok(Json(body)) => body,
        Err(JsonRejection::BytesRejection(rejection)) => {
            let (status, why) = crate::agent_api::unread(&rejection);
            return error(status, why);
        }
        Err(rejection) => return error(rejection.status(), rejection.body_text()),
    };
    let Some(session) = app.session(&id) else { return no_session() };
    let (outcome, held) = service::call(&app, app::Owner::Admin, &session, &body.tool, body.arguments).await;
    called(outcome, held)
}

/// An agent call's outcome as HTTP, for the administrator's page and the agent API (only an agent's token can be
/// revoked, so that is a bearer refusal), giving back what its answer holds once its bytes are written (see
/// `service::respond`). MCP answers in tool results of its own.
pub(crate) fn called(outcome: Called, held: Option<agent::Held>) -> Response {
    match outcome {
        Called::Done(result) => service::respond(StatusCode::OK, result, held),
        Called::Stopped => error(StatusCode::CONFLICT, "the session was stopped after a failure"),
        Called::Controlled => {
            error(StatusCode::CONFLICT, "a person holds control of this session; try again once they give it back")
        }
        Called::Failed(why) => {
            // Moved in: `json!` would copy it.
            let mut body = json!({});
            body["error"] = Value::String(why);
            service::respond(StatusCode::BAD_GATEWAY, body, held)
        }
        Called::Revoked => crate::agent_api::unauthenticated(),
        Called::Busy => busy(),
    }
}

async fn stats(State(app): State<Shared>, Login(login, _): Login, Path(id): Path<String>) -> Response {
    let Some(session) = app.session_for(app.principal(login), &id) else { return no_session() };
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
            secret_answer(listed)
        }
        Err(crate::tokens::CreateError::BadName) => {
            error(StatusCode::BAD_REQUEST, format!("name it in 1 to {} characters", crate::tokens::MAX_NAME))
        }
        Err(crate::tokens::CreateError::TooMany) => {
            error(StatusCode::CONFLICT, format!("at most {} tokens; revoke one first", crate::tokens::MAX_TOKENS))
        }
        Err(crate::tokens::CreateError::Unsaved(why)) => error(StatusCode::INTERNAL_SERVER_ERROR, why),
    }
}

/// Revokes a token, then stops every session started with it. The token is refused from now on whatever happens to the
/// stops; the revocation is written first, and if that fails the answer says so (it is retried, but a restart before
/// then accepts the token).
async fn revoke_token(State(app): State<Shared>, _: Administrator, Path(id): Path<String>) -> Response {
    let Some(id) = crate::tokens::parse_id(&id) else { return error(StatusCode::NOT_FOUND, "no such token") };
    detached(async move {
        let revoked = app.tokens.lock().unwrap().revoke(id);
        let unsaved = match revoked {
            None => return error(StatusCode::NOT_FOUND, "no such token"),
            Some(saved) => saved.err(),
        };
        tracing::info!(token = %crate::tokens::id_text(id), "agent token revoked");
        app.profiles.lock().unwrap().forget_token(id);
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
                format!(
                    "the token is refused, but its revocation could not be written; it is written again every few \
                     seconds, and a restart before then would accept the token: {why}"
                ),
            ),
            (None, false) => error(
                StatusCode::BAD_GATEWAY,
                format!("the token is revoked; stopping its sessions failed: {}", failures.join("; ")),
            ),
        }
    })
    .await
}

/// A profile refusal as HTTP: `423 Locked` while no password unlocked the profiles.
fn profile_refused(refused: crate::profiles::Refused) -> Response {
    let (status, why) = refused.answer();
    error(status, why)
}

/// The saved profiles (see `profiles`), and whether they are unlocked: never in the demo.
async fn list_profiles(State(app): State<Shared>, _: Administrator) -> Response {
    let unlocked = app.auth.lock().unwrap().kek().is_some();
    let profiles = app.profiles.lock().unwrap().list(app::Owner::Admin);
    let max = crate::profiles::MAX_PROFILES;
    Json(json!({"unlocked": unlocked, "profiles": profiles, "max": max})).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewProfile {
    name: String,
}

/// A new, empty profile: the first session from it starts with a fresh Chrome, and its end saves it.
async fn create_profile(State(app): State<Shared>, _: Administrator, Json(body): Json<NewProfile>) -> Response {
    let kek = app.auth.lock().unwrap().kek();
    let created = app.profiles.lock().unwrap().create(&body.name, kek.as_ref());
    match created {
        Ok(listed) => {
            tracing::info!(profile = listed["id"].as_str().unwrap_or_default(), "profile created");
            Json(listed).into_response()
        }
        Err(refused) => profile_refused(refused),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileChange {
    #[serde(default)]
    name: Option<String>,
    /// The agent tokens, by id, and the local MCP clients (`local`) that may start sessions from it.
    #[serde(default)]
    allowed: Option<Vec<String>>,
}

/// Renames a profile, or sets who besides the administrator may use it: only tokens that exist, and `local`.
async fn update_profile(
    State(app): State<Shared>,
    _: Administrator,
    Path(id): Path<String>,
    Json(body): Json<ProfileChange>,
) -> Response {
    if let Some(allowed) = &body.allowed {
        let tokens = app.tokens.lock().unwrap();
        let known =
            |entry: &String| entry == "local" || crate::tokens::parse_id(entry).is_some_and(|id| tokens.is_live(id));
        if !allowed.iter().all(known) {
            return error(StatusCode::BAD_REQUEST, "no such token");
        }
    }
    let updated = app.profiles.lock().unwrap().update(&id, body.name.as_deref(), body.allowed.as_deref());
    match updated {
        Ok(listed) => Json(listed).into_response(),
        Err(refused) => profile_refused(refused),
    }
}

/// Deletes a profile and its archive, unless a session uses it.
async fn delete_profile(State(app): State<Shared>, _: Administrator, Path(id): Path<String>) -> Response {
    let deleted = app.profiles.lock().unwrap().delete(&id);
    match deleted {
        Ok(()) => {
            tracing::info!(profile = %id, "profile deleted");
            Json(json!({"ok": true})).into_response()
        }
        Err(refused) => profile_refused(refused),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Unlock {
    password: String,
}

/// Unlocks the saved profiles with the password, after a sign-in that checked none (a passkey's): checked as a login
/// checks it, and counted toward the same backoff when it is wrong.
async fn unlock_profiles(
    State(app): State<Shared>,
    _: Administrator,
    Json(body): Json<Unlock>,
) -> Result<Response, Refusal> {
    let Some(salt) = app.auth.lock().unwrap().salt() else { return Ok(not_in_the_demo()) };
    let password = body.password;
    check_password(&app, password.clone()).await?;
    Ok(match tokio::task::spawn_blocking(move || keys::derive(&password, &salt)).await {
        Ok(kek) => {
            app.auth.lock().unwrap().unlock(kek);
            tracing::info!("saved profiles unlocked");
            Json(json!({"unlocked": true})).into_response()
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e),
    })
}

/// The current password, for a change of the second step: checked as a login checks it, off the async workers and
/// within the same bound on checks at once, and counted toward the same backoff when it is wrong.
async fn check_password(app: &Shared, password: String) -> Result<(), Refusal> {
    let hash = app.auth.lock().unwrap().begin_login()?;
    let shared = app.clone();
    tokio::task::spawn_blocking(move || {
        let matched = Auth::verify_password(&hash, &password);
        shared.auth.lock().unwrap().finish_password_check(matched)
    })
    .await
    .unwrap_or(Err(Refusal::WrongPassword))
}

/// Makes a change of the second step for `login` and ends every other login, as a change of how one signs in must;
/// the streams of the logins ended are closed once the auth lock is let go (see [`App`] for the lock order).
fn change_second_step<T>(
    app: &App,
    login: LoginKey,
    change: impl FnOnce(&mut crate::second_step::SecondStep) -> Result<T, Refusal>,
) -> Result<T, Refusal> {
    let (changed, ended) = {
        let mut auth = app.auth.lock().unwrap();
        // Checked again under the lock: the request's login may have ended (another browser's change ended it) while
        // its body arrived or its password was checked.
        if !auth.is_live(&login) {
            return Err(Refusal::Unauthenticated);
        }
        let changed = change(auth.second_mut()?)?;
        (changed, auth.end_other_logins(login))
    };
    for other in ended {
        app.end_login(other, crate::app::ENDED);
    }
    Ok(changed)
}

/// An answer that holds a secret (the app's, or recovery codes): never stored by the browser or anything between.
fn secret_answer(body: Value) -> Response {
    unstored(StatusCode::OK, body)
}

/// What the second step holds, for the Security page.
async fn security(State(app): State<Shared>, _: Administrator) -> Result<Response, Refusal> {
    let status = app.auth.lock().unwrap().second_mut().and_then(|second| second.status())?;
    Ok(Json(status).into_response())
}

/// Draws a secret for the authenticator app, which turns nothing on until a code from the app confirms it.
async fn begin_app(State(app): State<Shared>, Administrator(login): Administrator) -> Result<Response, Refusal> {
    let (secret, uri) = app.auth.lock().unwrap().second_mut().map(|second| second.begin_app(login))?;
    Ok(secret_answer(json!({"secret": secret, "uri": uri})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfirmApp {
    code: String,
    /// The current password, which replacing an app that is on takes.
    #[serde(default)]
    password: Option<String>,
}

/// Turns the authenticator app on, or replaces it, once a code from the app shows it was added there. The recovery
/// codes are answered when the second step was off until now: this is the one time they are shown.
async fn confirm_app(
    State(app): State<Shared>,
    Administrator(login): Administrator,
    Json(body): Json<ConfirmApp>,
) -> Result<Response, Refusal> {
    let checked = body.password.is_some();
    if let Some(password) = body.password {
        check_password(&app, password).await?;
    }
    let codes = change_second_step(&app, login, |second| second.confirm_app(login, &body.code, checked))?;
    tracing::info!("the authenticator app was turned on; every other login ended");
    Ok(secret_answer(json!({"recovery_codes": codes})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasswordBody {
    password: String,
}

async fn remove_app(
    State(app): State<Shared>,
    Administrator(login): Administrator,
    Json(body): Json<PasswordBody>,
) -> Result<Response, Refusal> {
    check_password(&app, body.password).await?;
    change_second_step(&app, login, |second| second.remove_app())?;
    tracing::info!("the authenticator app was turned off; every other login ended");
    Ok(Json(json!({"ok": true})).into_response())
}

/// New recovery codes in place of those left, shown in this answer only.
async fn new_recovery_codes(
    State(app): State<Shared>,
    Administrator(login): Administrator,
    Json(body): Json<PasswordBody>,
) -> Result<Response, Refusal> {
    check_password(&app, body.password).await?;
    let codes = change_second_step(&app, login, |second| second.new_recovery_codes())?;
    tracing::info!("new recovery codes replaced the ones left; every other login ended");
    Ok(secret_answer(json!({"recovery_codes": codes})))
}

/// Begins adding a passkey, which takes the current password: a passkey alone opens a login.
async fn begin_passkey(
    State(app): State<Shared>,
    Administrator(login): Administrator,
    Json(body): Json<PasswordBody>,
) -> Result<Response, Refusal> {
    check_password(&app, body.password).await?;
    let options = {
        let mut auth = app.auth.lock().unwrap();
        // The login may have ended while its password was checked.
        if auth.is_live(&login) {
            auth.second_mut().and_then(|second| second.begin_passkey(login))
        } else {
            Err(Refusal::Unauthenticated)
        }
    }?;
    Ok(Json(json!({"options": options})).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewPasskey {
    credential: webauthn_rs::prelude::RegisterPublicKeyCredential,
}

/// Adds the passkey begun by this login. The recovery codes are answered when the second step was off until now.
async fn finish_passkey(
    State(app): State<Shared>,
    Administrator(login): Administrator,
    Json(body): Json<NewPasskey>,
) -> Result<Response, Refusal> {
    let codes = change_second_step(&app, login, |second| second.finish_passkey(login, &body.credential))?;
    tracing::info!("a passkey was added; every other login ended");
    Ok(secret_answer(json!({"recovery_codes": codes})))
}

async fn remove_passkey(
    State(app): State<Shared>,
    Administrator(login): Administrator,
    Path(id): Path<String>,
    Json(body): Json<PasswordBody>,
) -> Result<Response, Refusal> {
    check_password(&app, body.password).await?;
    change_second_step(&app, login, |second| second.remove_passkey(&id))?;
    tracing::info!("a passkey was removed; every other login ended");
    Ok(Json(json!({"ok": true})).into_response())
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
        Err(r) => return r.into_response(),
    };
    if query.viewer.len() != 32 || !query.viewer.bytes().all(|b| b.is_ascii_hexdigit()) {
        return error(StatusCode::BAD_REQUEST, "the viewer token is 32 hex characters");
    }
    let Some(session) = app.session_for(app.principal(login), &id) else { return no_session() };
    let (place, deadline) = match app.config.demo {
        None => (None, None),
        Some(_) => {
            let Some(deadline) = crate::demo::deadline(&app, login) else {
                return error(StatusCode::GONE, "your visit is over");
            };
            match crate::demo::viewer_place(&session) {
                Some(place) => (Some(place), Some(deadline)),
                None => return error(StatusCode::TOO_MANY_REQUESTS, "this demo is already open in another tab"),
            }
        }
    };
    upgrade
        .max_message_size(display_protocol::MAX_WEBSOCKET_MESSAGE)
        .max_frame_size(display_protocol::MAX_WEBSOCKET_MESSAGE)
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

async fn download(
    State(app): State<Shared>,
    Login(login, _): Login,
    headers: HeaderMap,
    Path((id, ticket)): Path<(String, String)>,
) -> Response {
    // The admin page's own navigation; another site never carries the Strict cookie, and is refused here as well.
    if headers.get("sec-fetch-site").is_some_and(|v| v.as_bytes() != b"same-origin") {
        return Refusal::Forbidden.into_response();
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
    let Some(slot) = app::Place::take(app.downloads.clone(), MAX_DOWNLOADS) else {
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
    /// One of the [`MAX_DOWNLOADS`] places.
    _slot: app::Place<Arc<std::sync::atomic::AtomicUsize>>,
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
    const ADMINISTRATOR_ROUTES: [(&str, &str, &str); 23] = [
        ("POST", "/api/sessions", r#"{"url": "https://a/"}"#),
        ("DELETE", "/api/sessions/abc", ""),
        ("PUT", "/api/sessions/abc/settings", r#"{"bitrate_kbps": 2500, "max_fps": 60}"#),
        ("PUT", "/api/sessions/abc/network", r#"{"latency_ms": 0, "loss_percent": 0, "bandwidth_kbps": 0}"#),
        ("POST", "/api/sessions/abc/agent", r#"{"tool": "list_windows"}"#),
        ("GET", "/api/tokens", ""),
        ("POST", "/api/tokens", r#"{"name": "a"}"#),
        ("DELETE", "/api/tokens/abc", ""),
        ("GET", "/api/security", ""),
        ("POST", "/api/security/app", ""),
        ("POST", "/api/security/app/confirm", r#"{"code": "000000"}"#),
        ("POST", "/api/security/passkeys/finish", "{}"),
        ("GET", "/api/profiles", ""),
        ("POST", "/api/profiles", r#"{"name": "a"}"#),
        ("PUT", "/api/profiles/abc", r#"{"name": "b"}"#),
        ("DELETE", "/api/profiles/abc", ""),
        ("GET", "/api/egresses", ""),
        ("GET", "/api/egresses/options", ""),
        ("POST", "/api/egresses", r#"{"name": "a", "proxy": "http://p.example:8080"}"#),
        ("PATCH", "/api/egresses/abc", r#"{"name": "b"}"#),
        ("DELETE", "/api/egresses/abc", ""),
        ("POST", "/api/egresses/check", r#"{"proxy": "http://p.example:8080"}"#),
        ("POST", "/api/egresses/abc/check", ""),
    ];

    /// The admin's own address in [`app::test_app`].
    const ORIGIN: &str = "https://localhost:9443";

    /// `body` (none when it is empty) to `path` by `method`, as the admin's own page sends it from `origin`, with
    /// `login`'s cookie and CSRF token if any: the status, the headers, and the JSON answered (null for none).
    async fn request(
        app: &Shared,
        method: &str,
        path: &str,
        origin: &str,
        body: impl ToString,
        login: Option<(&str, &str)>,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::ORIGIN, origin)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some((cookie, csrf)) = login {
            request =
                request.header(header::COOKIE, format!("{}={cookie}", auth::COOKIE)).header(auth::CSRF_HEADER, csrf);
        }
        let response = router(app.clone()).oneshot(request.body(Body::from(body.to_string())).unwrap()).await.unwrap();
        let (parts, body) = response.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        (parts.status, parts.headers, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    /// The login cookie an answer sets, if any.
    fn cookie_set(headers: &HeaderMap) -> Option<String> {
        let set = headers.get(header::SET_COOKIE)?.to_str().unwrap();
        Some(set.strip_prefix(&format!("{}=", auth::COOKIE)).and_then(|c| c.split(';').next()).unwrap().to_owned())
    }

    #[tokio::test]
    async fn administrator_routes_need_a_login_and_refuse_every_demo_visitor() {
        let demo = app::test_app(true);
        let (cookie, csrf) = demo.auth.lock().unwrap().open_login();
        for (method, path, body) in ADMINISTRATOR_ROUTES {
            assert_eq!(request(&demo, method, path, ORIGIN, body, None).await.0, StatusCode::UNAUTHORIZED, "{path}");
            let status = request(&demo, method, path, ORIGIN, body, Some((&cookie, &csrf))).await.0;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
        }
        // Outside the demo, the same login reaches them: no such session or token here, or a body it took.
        let app = app::test_app(false);
        let (cookie, csrf) = app.auth.lock().unwrap().open_login();
        for (method, path, body) in ADMINISTRATOR_ROUTES {
            assert_eq!(request(&app, method, path, ORIGIN, body, None).await.0, StatusCode::UNAUTHORIZED, "{path}");
            let status = request(&app, method, path, ORIGIN, body, Some((&cookie, &csrf))).await.0;
            assert!(![StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN].contains(&status), "{method} {path}: {status}");
        }
    }

    #[tokio::test]
    async fn the_administrator_s_tool_call_fits_the_largest_script_and_a_larger_body_is_refused_as_json() {
        let app = app::test_app(false);
        let (cookie, csrf) = app.auth.lock().unwrap().open_login();
        let call =
            |body: String| request(&app, "POST", "/api/sessions/none/agent", ORIGIN, body, Some((&cookie, &csrf)));
        // run_script's longest script, every code point written as a surrogate pair's two escapes.
        let script = r"\ud83d\ude00".repeat(65_536);
        let longest = format!(r#"{{"tool":"run_script","arguments":{{"script":"{script}","timeout_ms":120000}}}}"#);
        let (status, _, body) = call(longest).await;
        assert_eq!((status, &body["error"]), (StatusCode::NOT_FOUND, &json!("no such session")), "read in full");
        let larger =
            format!(r#"{{"tool":"run_script","arguments":{{"script":"{}"}}}}"#, "a".repeat(crate::tools::BODY_LIMIT));
        let (status, _, body) = call(larger).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(body["error"].as_str().is_some_and(|e| e.contains("larger than")), "{body}");
        let (status, _, body) = call(r#"{"tool": 1}"#.into()).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body["error"].is_string(), "{body}");
    }

    #[tokio::test]
    async fn saved_profiles_unlock_with_the_password_and_a_locked_or_used_one_starts_no_session() {
        let (app, cookie, csrf) = enrolled().await;
        let (status, _, made) =
            request(&app, "POST", "/api/profiles", ORIGIN, json!({"name": "Personal"}), Some((&cookie, &csrf))).await;
        assert_eq!(status, StatusCode::OK, "setting the password unlocked them: {made}");
        let id = made["id"].as_str().unwrap().to_owned();
        // A restart, then a sign-in that checked no password (a passkey's): the profiles are locked.
        let mut restarted = Auth::load(&app.config.state_dir, true).unwrap();
        let (cookie, csrf) = restarted.open_login();
        *app.auth.lock().unwrap() = restarted;
        let login = Some((cookie.as_str(), csrf.as_str()));
        let (_, _, listed) = request(&app, "GET", "/api/profiles", ORIGIN, "", login).await;
        assert_eq!((&listed["unlocked"], &listed["profiles"][0]["name"]), (&json!(false), &json!("Personal")));
        let start = json!({"url": "https://a.test/", "profile": id});
        let (status, _, refused) = request(&app, "POST", "/api/sessions", ORIGIN, start.clone(), login).await;
        assert_eq!(status, StatusCode::LOCKED, "{refused}");
        let (status, _, _) = request(&app, "POST", "/api/profiles", ORIGIN, json!({"name": "Work"}), login).await;
        assert_eq!(status, StatusCode::LOCKED);
        let (status, _, wrong) =
            request(&app, "POST", "/api/profiles/unlock", ORIGIN, json!({"password": "wrong one!!!"}), login).await;
        assert_eq!((status, &wrong["reason"]), (StatusCode::FORBIDDEN, &json!("password")));
        let unlock = json!({"password": "correct horse battery"});
        assert_eq!(request(&app, "POST", "/api/profiles/unlock", ORIGIN, unlock, login).await.0, StatusCode::OK);
        assert_eq!(request(&app, "GET", "/api/profiles", ORIGIN, "", login).await.2["unlocked"], true);
        // In use by another session: neither started from nor deleted.
        let kek = app.auth.lock().unwrap().kek();
        let held = app.profiles.lock().unwrap().reserve(app::Owner::Admin, &id, "0123456789ab", kek.as_ref()).unwrap();
        assert_eq!(request(&app, "POST", "/api/sessions", ORIGIN, start, login).await.0, StatusCode::CONFLICT);
        let path = format!("/api/profiles/{id}");
        assert_eq!(request(&app, "DELETE", &path, ORIGIN, "", login).await.0, StatusCode::CONFLICT);
        app.profiles.lock().unwrap().release(&held.id, "0123456789ab");
        let (status, _, _) = request(&app, "PUT", &path, ORIGIN, json!({"allowed": ["0123456789abcdef"]}), login).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "only a token that exists");
        let (status, _, changed) =
            request(&app, "PUT", &path, ORIGIN, json!({"name": "Mine", "allowed": ["local"]}), login).await;
        assert_eq!(
            (status, &changed["name"], &changed["allowed"]),
            (StatusCode::OK, &json!("Mine"), &json!(["local"]))
        );
        assert_eq!(request(&app, "DELETE", &path, ORIGIN, "", login).await.0, StatusCode::OK);
        assert_eq!(request(&app, "DELETE", &path, ORIGIN, "", login).await.0, StatusCode::NOT_FOUND);
    }

    /// An installation whose password is set, and the login that setting it opened, with the login's own cookie.
    async fn enrolled() -> (app::TestApp, String, String) {
        let app = app::test_app(false);
        let body = json!({"password": "correct horse battery"});
        let (status, headers, body) = request(&app, "POST", "/api/enroll", ORIGIN, body, None).await;
        assert_eq!(status, StatusCode::OK);
        let cookie = cookie_set(&headers).unwrap();
        assert_eq!(headers[header::SET_COOKIE], auth::set_cookie(&cookie), "the login's own cookie");
        let csrf = body["csrf"].as_str().unwrap().to_owned();
        (app, cookie, csrf)
    }

    #[tokio::test]
    async fn the_authenticator_app_turns_on_signs_in_with_its_code_and_turns_off_with_the_password() {
        let (app, cookie, csrf) = enrolled().await;
        let login = Some((cookie.as_str(), csrf.as_str()));
        let other = app.auth.lock().unwrap().open_login();
        let (status, _, drawn) = request(&app, "POST", "/api/security/app", ORIGIN, "null", login).await;
        assert_eq!(status, StatusCode::OK);
        assert!(drawn["uri"].as_str().unwrap().starts_with("otpauth://totp/easi.run:admin?secret="), "{drawn}");
        let code = app.auth.lock().unwrap().second_mut().unwrap().test_code(0);
        let (status, _, confirmed) =
            request(&app, "POST", "/api/security/app/confirm", ORIGIN, json!({"code": code}), login).await;
        assert_eq!(status, StatusCode::OK, "{confirmed}");
        assert_eq!(confirmed["recovery_codes"].as_array().map(Vec::len), Some(10));
        assert!(!app.auth.lock().unwrap().is_live(&auth::key_of(&other.0)), "the other login ended");
        assert!(app.auth.lock().unwrap().is_live(&auth::key_of(&cookie)), "this one stays");

        // The password now answers with a pending step and no cookie; its code opens the login.
        let password = json!({"password": "correct horse battery"});
        let (status, headers, answer) = request(&app, "POST", "/api/login", ORIGIN, password.clone(), None).await;
        assert_eq!((status, cookie_set(&headers)), (StatusCode::OK, None), "{answer}");
        let step = answer["second_step"]["step"].as_str().unwrap().to_owned();
        assert_eq!(answer["second_step"]["app"], true);
        let answered =
            |code: &str| request(&app, "POST", "/api/login/code", ORIGIN, json!({"step": step, "code": code}), None);
        let (status, headers, _) = answered(&code).await;
        assert_eq!((status, cookie_set(&headers)), (StatusCode::UNAUTHORIZED, None), "the confirming code is used");
        let next = app.auth.lock().unwrap().second_mut().unwrap().test_code(1);
        let (status, headers, signed_in) = answered(&next).await;
        assert_eq!(status, StatusCode::OK);
        assert!(cookie_set(&headers).is_some() && signed_in["csrf"].is_string());
        let (status, _, again) = answered(&next).await;
        assert_eq!((status, &again["reason"]), (StatusCode::UNAUTHORIZED, &json!("again")), "single use");

        // Turning it off takes the current password, refused without ending the login.
        let wrong = json!({"password": "not my password"});
        let (status, _, refused) = request(&app, "POST", "/api/security/app/remove", ORIGIN, wrong, login).await;
        assert_eq!((status, &refused["reason"]), (StatusCode::FORBIDDEN, &json!("password")));
        let (status, _, _) = request(&app, "POST", "/api/security/app/remove", ORIGIN, json!({}), login).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "no password at all");
        let (status, _, _) = request(&app, "POST", "/api/security/app/remove", ORIGIN, password.clone(), login).await;
        assert_eq!(status, StatusCode::OK);
        let (status, headers, answer) = request(&app, "POST", "/api/login", ORIGIN, password, None).await;
        let set = cookie_set(&headers);
        assert!(status == StatusCode::OK && set.is_some() && answer["csrf"].is_string(), "the password alone again");
    }

    #[tokio::test]
    async fn every_change_of_the_second_step_but_turning_the_app_on_takes_the_current_password() {
        let (app, cookie, csrf) = enrolled().await;
        let login = Some((cookie.as_str(), csrf.as_str()));
        let wrong = json!({"password": "not my password"});
        for path in [
            "/api/security/passkeys",
            "/api/security/passkeys/abc/remove",
            "/api/security/app/remove",
            "/api/security/recovery",
        ] {
            let (status, _, body) = request(&app, "POST", path, ORIGIN, wrong.clone(), login).await;
            assert_eq!((status, &body["reason"]), (StatusCode::FORBIDDEN, &json!("password")), "{path}");
        }
        let right = json!({"password": "correct horse battery"});
        let (status, _, body) = request(&app, "POST", "/api/security/passkeys", ORIGIN, right, login).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["options"]["publicKey"]["rp"]["id"], "localhost");
        let (_, _, shown) = request(&app, "GET", "/api/status", ORIGIN, "", None).await;
        assert_eq!(shown["passkey"], false, "none added yet");
    }

    #[tokio::test]
    async fn a_passkey_signs_in_in_place_of_the_password_and_leaves_the_profiles_locked() {
        let (app, cookie, csrf) = enrolled().await;
        let login = Some((cookie.as_str(), csrf.as_str()));
        let mut key = crate::second_step::tests::SoftKey::new();
        let password = json!({"password": "correct horse battery"});
        let (_, _, begun) = request(&app, "POST", "/api/security/passkeys", ORIGIN, password, login).await;
        let added = json!({"credential": key.registration(&begun["options"])});
        let (status, _, added) = request(&app, "POST", "/api/security/passkeys/finish", ORIGIN, added, login).await;
        assert_eq!((status, added["recovery_codes"].as_array().map(Vec::len)), (StatusCode::OK, Some(10)), "{added}");
        // After a restart the login screen offers it, and it signs in alone; no password derived the profiles' key.
        let mut restarted = Auth::load(&app.config.state_dir, true).unwrap();
        restarted.offer_passkeys(ORIGIN);
        *app.auth.lock().unwrap() = restarted;
        assert_eq!(request(&app, "GET", "/api/status", ORIGIN, "", None).await.2["passkey"], true);
        let (status, _, begun) = request(&app, "POST", "/api/login/passkey", ORIGIN, "", None).await;
        assert_eq!(status, StatusCode::OK, "{begun}");
        let signed = key.assertion(&begun["options"]);
        let answer = json!({"ceremony": begun["ceremony"], "credential": signed});
        let finish = |answer: Value| request(&app, "POST", "/api/login/passkey/finish", ORIGIN, answer, None);
        let (status, headers, opened) = finish(answer.clone()).await;
        assert_eq!(status, StatusCode::OK, "{opened}");
        let (cookie, csrf) = (cookie_set(&headers).unwrap(), opened["csrf"].as_str().unwrap().to_owned());
        let login = Some((cookie.as_str(), csrf.as_str()));
        assert_eq!(request(&app, "GET", "/api/me", ORIGIN, "", login).await.2["csrf"], csrf.as_str());
        assert_eq!(request(&app, "GET", "/api/profiles", ORIGIN, "", login).await.2["unlocked"], false);
        // From another address, refused before anything is checked.
        let (status, _, _) = request(&app, "POST", "/api/login/passkey", "https://127.0.0.1:9443", "", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        // The same answer to a new ceremony signs in nobody, and counts toward the backoff as a wrong password does.
        for _ in 0..5 {
            let (_, _, begun) = request(&app, "POST", "/api/login/passkey", ORIGIN, "", None).await;
            let replayed = json!({"ceremony": begun["ceremony"], "credential": signed});
            let (status, headers, _) = finish(replayed).await;
            assert_eq!((status, cookie_set(&headers)), (StatusCode::UNAUTHORIZED, None));
        }
        let (status, _, _) = request(&app, "POST", "/api/login/passkey", ORIGIN, "", None).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn a_change_of_the_second_step_by_a_login_that_ended_meanwhile_is_refused() {
        let app = app::test_app(false);
        let (cookie, _) = app.auth.lock().unwrap().open_login();
        let login = auth::key_of(&cookie);
        app.auth.lock().unwrap().logout(login);
        let changed = change_second_step(&app, login, |second| second.new_recovery_codes());
        assert_eq!(changed.err(), Some(Refusal::Unauthenticated));
    }

    #[tokio::test]
    async fn the_demo_has_no_second_step() {
        let demo = app::test_app(true);
        for (path, body) in
            [("/api/login/code", json!({"step": "a", "code": "123456"})), ("/api/login/passkey", json!(null))]
        {
            assert_eq!(request(&demo, "POST", path, ORIGIN, body, None).await.0, StatusCode::FORBIDDEN, "{path}");
        }
    }

    #[tokio::test]
    async fn a_page_asked_for_at_another_address_is_sent_on_to_the_admin_s_own() {
        let app = app::test_app(false);
        let get = |app: &app::TestApp, host: &str, path: &str| {
            let request = Request::get(path).header(header::HOST, host).body(Body::empty()).unwrap();
            let router = router((*app).clone());
            async move { router.oneshot(request).await.unwrap() }
        };
        for (host, path, location) in [
            ("127.0.0.1:9443", "/", "https://localhost:9443/"),
            ("localhost:9444", "/index.html?x=1", "https://localhost:9443/index.html?x=1"),
            ("203.0.113.7", "/", "https://localhost:9443/"),
            // Never anywhere else: the host and path a request names are not where it goes.
            ("evil.example", "//evil.example/x", "https://localhost:9443//evil.example/x"),
            ("localhost:9443@evil.example", "/", "https://localhost:9443/"),
        ] {
            let response = get(&app, host, path).await;
            assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT, "{host}{path}");
            assert_eq!(response.headers()[header::LOCATION], location, "{host}{path}");
        }
        // The admin's own address, however it is spelled, is served; so is the demo at any.
        for host in ["localhost:9443", "LOCALHOST:9443"] {
            assert_eq!(get(&app, host, "/").await.status(), StatusCode::NOT_FOUND, "{host}: no page in the test");
        }
        let demo = app::test_app(true);
        assert_eq!(get(&demo, "127.0.0.1:9443", "/").await.status(), StatusCode::NOT_FOUND);
        // The API is never sent on: a state change from another origin is refused instead, naming the admin's own.
        let request = Request::get("/api/status").header(header::HOST, "127.0.0.1:9443").body(Body::empty()).unwrap();
        assert_eq!(router(app.clone()).oneshot(request).await.unwrap().status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn the_first_password_is_set_once_and_only_from_the_exact_origin() {
        let app = app::test_app(false);
        let enroll = json!({"password": "correct horse battery"});
        for origin in ["https://127.0.0.1:9443", "https://localhost:9444", "https://evil.example", "null"] {
            for (path, body) in [("/api/enroll", &enroll), ("/api/login", &enroll)] {
                let (status, _, body) = request(&app, "POST", path, origin, body, None).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{path} from {origin}");
                assert_eq!(
                    (&body["reason"], &body["origin"]),
                    (&json!("wrong-origin"), &json!("https://localhost:9443"))
                );
            }
        }
        assert!(!app.auth.lock().unwrap().enrolled(), "none of these set it");
        let extra = json!({"password": "correct horse battery", "token": "x"});
        assert!(request(&app, "POST", "/api/enroll", ORIGIN, extra, None).await.0.is_client_error());
        let (status, _, body) = request(&app, "POST", "/api/enroll", ORIGIN, &enroll, None).await;
        assert_eq!(status, StatusCode::OK, "set at the admin's own address");
        assert!(body["csrf"].is_string(), "{body}");
        let again = json!({"password": "another long password"});
        let (status, _, body) = request(&app, "POST", "/api/enroll", ORIGIN, again, None).await;
        assert_eq!((status, &body["error"]), (StatusCode::FORBIDDEN, &json!("forbidden")), "never replaced");
        // The demo never enrolls.
        let demo = app::test_app(true);
        let (status, _, _) = request(&demo, "POST", "/api/enroll", ORIGIN, &enroll, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn the_login_routes_check_the_login_before_the_body() {
        let app = app::test_app(false);
        for (method, path) in [
            ("POST", "/api/demo/switch"),
            ("POST", "/api/logout"),
            ("GET", "/api/sessions"),
            ("GET", "/api/egresses"),
            ("POST", "/api/egresses"),
            ("POST", "/api/egresses/check"),
            ("POST", "/api/sessions/abc/control"),
            ("POST", "/api/sessions/abc/release"),
            ("GET", "/api/sessions/abc/stats"),
            ("GET", "/api/sessions/abc/files/abc"),
        ] {
            let status = request(&app, method, path, ORIGIN, "not json", None).await.0;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
        }
        let (cookie, csrf) = app.auth.lock().unwrap().open_login();
        let control = request(&app, "POST", "/api/sessions/abc/control", ORIGIN, "not json", Some((&cookie, &csrf)));
        assert_eq!(control.await.0, StatusCode::BAD_REQUEST, "then the body");
    }

    /// A controller on a Unix socket in `dir` that answers each `check_egress` with the next of `outcomes` (an error
    /// line when none is left), refuses every `create_session`, and stops anything; and every request it got.
    fn checking_controller(
        dir: &std::path::Path,
        outcomes: Vec<Value>,
    ) -> (crate::controller::Controller, Arc<std::sync::Mutex<Vec<Value>>>) {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
        let path = dir.join("controller.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (log, outcomes) =
            (asked.clone(), Arc::new(std::sync::Mutex::new(std::collections::VecDeque::from(outcomes))));
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let mut stream = tokio::io::BufReader::new(stream);
                let mut line = String::new();
                stream.read_line(&mut line).await.unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                log.lock().unwrap().push(request.clone());
                let answer = match request["op"].as_str().unwrap() {
                    "check_egress" => match outcomes.lock().unwrap().pop_front() {
                        Some(outcome) => json!({"ok": true, "outcome": outcome}),
                        None => {
                            json!({"ok": false, "error": "refused: 4 egress checks are under way; try again shortly"})
                        }
                    },
                    "create_session" => json!({"ok": false, "error": "refused in this test"}),
                    _ => json!({"ok": true}),
                };
                stream.get_mut().write_all(format!("{answer}\n").as_bytes()).await.unwrap();
            }
        });
        (crate::controller::Controller::new(path), asked)
    }

    fn passed(country: &str) -> Value {
        json!({"ok": true, "latency_ms": 321, "ip": "203.0.113.9", "country": country})
    }

    fn failed() -> Value {
        json!({"ok": false, "reason": "auth", "detail": "the proxy refused the user name or password"})
    }

    #[tokio::test]
    async fn an_egress_is_saved_only_once_its_check_passed_and_is_listed_without_its_credentials() {
        let socket = crate::private::test_dir("api-egress");
        let (controller, asked) = checking_controller(&socket, vec![failed(), passed("BR"), passed("DE")]);
        let app = app::test_app_with(false, controller);
        let (cookie, csrf) = app.auth.lock().unwrap().open_login();
        let login = Some((cookie.as_str(), csrf.as_str()));
        let new = json!({"name": "home", "proxy": "http://user-{session}:hunter2@proxy.example.net:8080"});
        let (status, headers, refused) = request(&app, "POST", "/api/egresses", ORIGIN, new.clone(), login).await;
        assert_eq!(
            (status, &refused["reason"], &headers[header::CACHE_CONTROL]),
            (StatusCode::UNPROCESSABLE_ENTITY, &json!("auth"), &HeaderValue::from_static("no-store"))
        );
        assert_eq!(refused["detail"], "the proxy refused the user name or password");
        assert_eq!(request(&app, "GET", "/api/egresses", ORIGIN, "", login).await.2["egresses"], json!([]));
        let (status, _, made) = request(&app, "POST", "/api/egresses", ORIGIN, new, login).await;
        assert_eq!(status, StatusCode::OK, "{made}");
        assert_eq!(
            (&made["country"], &made["country_name"], &made["latency_ms"]),
            (&json!("BR"), &json!("Brazil"), &json!(321))
        );
        assert_eq!((&made["timezone"], &made["languages"][0]), (&json!("America/Sao_Paulo"), &json!("pt-BR")));
        // The check was of the upstream a session would use: its placeholder filled with a session-like id.
        let checked = asked.lock().unwrap()[1]["upstream"]["username"].as_str().unwrap().to_owned();
        assert!(checked.starts_with("user-") && checked.len() == 17 && !checked.contains('{'), "{checked}");
        let (_, headers, listed) = request(&app, "GET", "/api/egresses", ORIGIN, "", login).await;
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        let shown = listed.to_string();
        assert!(!shown.contains("hunter2") && !shown.contains("user-"), "{shown}");
        assert_eq!((&listed["egresses"][0]["has_user"], &listed["egresses"][0]["in_use"]), (&json!(true), &json!(0)));
        // Stated place wins over the suggestion; a field given as null is refused.
        let id = made["id"].as_str().unwrap().to_owned();
        let path = format!("/api/egresses/{id}");
        let (status, _, changed) = request(
            &app,
            "PATCH",
            &path,
            ORIGIN,
            json!({"proxy": "socks5://u:pw@proxy.example.net:1080", "timezone": "UTC"}),
            login,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{changed}");
        assert_eq!(
            (&changed["scheme"], &changed["country"], &changed["timezone"], &changed["languages"][0]),
            (&json!("socks5"), &json!("DE"), &json!("UTC"), &json!("de-DE"))
        );
        let (status, _, _) = request(&app, "PATCH", &path, ORIGIN, json!({"proxy": null}), login).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        // No check left in the fake: the controller is busy, and the proxy stays as it was.
        let (status, headers, _) =
            request(&app, "PATCH", &path, ORIGIN, json!({"proxy": "http://other.example:3128"}), login).await;
        assert_eq!(
            (status, &headers[header::RETRY_AFTER]),
            (StatusCode::SERVICE_UNAVAILABLE, &HeaderValue::from_static("2"))
        );
        assert_eq!(request(&app, "GET", "/api/egresses", ORIGIN, "", login).await.2["egresses"][0]["scheme"], "socks5");
        let (status, _, refused) = request(
            &app,
            "POST",
            "/api/egresses",
            ORIGIN,
            json!({"name": "x", "proxy": "ftp://hunter2@p.example:21"}),
            login,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(!refused.to_string().contains("hunter2"), "{refused}");
        let options = request(&app, "GET", "/api/egresses/options", ORIGIN, "", login).await.2;
        assert!(options["timezones"].as_array().unwrap().contains(&json!("Europe/Berlin")));
        assert_eq!(options["languages"][0], json!({"tag": "en-US", "name": "English (United States)"}));
    }

    #[tokio::test]
    async fn a_check_answers_what_it_found_and_saves_nothing() {
        let socket = crate::private::test_dir("api-egress");
        let (controller, _) = checking_controller(&socket, vec![passed("BR"), failed(), passed("NL")]);
        let app = app::test_app_with(false, controller);
        let (cookie, csrf) = app.auth.lock().unwrap().open_login();
        let login = Some((cookie.as_str(), csrf.as_str()));
        let draft = json!({"proxy": "http://u:pw@proxy.example.net:8080"});
        let (status, _, found) = request(&app, "POST", "/api/egresses/check", ORIGIN, draft.clone(), login).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            found["exit"],
            json!({"ip": "203.0.113.9", "country": "BR", "country_name": "Brazil", "region": null, "city": null})
        );
        assert_eq!(found["suggested"]["timezone"], "America/Sao_Paulo");
        let (status, _, found) = request(&app, "POST", "/api/egresses/check", ORIGIN, draft, login).await;
        assert_eq!((status, found), (StatusCode::OK, failed()));
        assert_eq!(request(&app, "GET", "/api/egresses", ORIGIN, "", login).await.2["egresses"], json!([]));
        assert_eq!(
            request(&app, "POST", "/api/egresses/0123456789abcdef/check", ORIGIN, "", login).await.0,
            StatusCode::NOT_FOUND
        );
        // From another origin, or without the CSRF token, a check is refused before it runs.
        let draft = json!({"proxy": "http://proxy.example.net:8080"});
        let (status, _, _) =
            request(&app, "POST", "/api/egresses/check", "https://evil.example", draft.clone(), login).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _, _) =
            request(&app, "POST", "/api/egresses/check", ORIGIN, draft, Some((&cookie, "wrong"))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn a_start_through_an_egress_checks_its_very_upstream_first_and_a_held_egress_is_never_deleted() {
        let socket = crate::private::test_dir("api-egress");
        let (controller, asked) = checking_controller(&socket, vec![passed("BR"), failed(), passed("BR")]);
        let app = app::test_app_with(false, controller);
        std::fs::create_dir_all(&app.config.streams_dir).unwrap();
        let (cookie, csrf) = app.auth.lock().unwrap().open_login();
        let login = Some((cookie.as_str(), csrf.as_str()));
        let new = json!({"name": "home", "proxy": "http://user-{session}:pw@proxy.example.net:8080"});
        let id = request(&app, "POST", "/api/egresses", ORIGIN, new, login).await.2["id"].as_str().unwrap().to_owned();
        let start = json!({"url": "https://a.test/", "egress": id});
        let (status, _, refused) = request(&app, "POST", "/api/sessions", ORIGIN, start.clone(), login).await;
        assert_eq!(status, StatusCode::FAILED_DEPENDENCY, "{refused}");
        assert_eq!(
            (&refused["reason"], &refused["detail"]),
            (&json!("auth"), &json!("the proxy refused the user name or password"))
        );
        let ops = |asked: &Arc<std::sync::Mutex<Vec<Value>>>| {
            asked.lock().unwrap().iter().map(|r| r["op"].as_str().unwrap().to_owned()).collect::<Vec<_>>()
        };
        assert_eq!(ops(&asked), ["check_egress", "check_egress"], "no session container was asked for");
        assert_eq!(
            request(&app, "GET", "/api/egresses", ORIGIN, "", login).await.2["egresses"][0]["in_use"],
            0,
            "given back"
        );
        // A pass: the session is asked for with the very upstream that was checked (the fake refuses it).
        let (status, _, _) = request(&app, "POST", "/api/sessions", ORIGIN, start, login).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        let requests = asked.lock().unwrap().clone();
        let (checked, created) = (&requests[2], &requests[3]);
        assert_eq!((checked["op"].as_str(), created["op"].as_str()), (Some("check_egress"), Some("create_session")));
        let session = created["session"].as_str().unwrap();
        assert_eq!(checked["upstream"], created["egress"]);
        assert_eq!(created["egress"]["username"], format!("user-{session}"));
        assert_eq!(created["timezone"], "America/Sao_Paulo");
        // An unknown egress starts nothing.
        let unknown = json!({"url": "https://a.test/", "egress": "0123456789abcdef"});
        assert_eq!(request(&app, "POST", "/api/sessions", ORIGIN, unknown, login).await.0, StatusCode::BAD_REQUEST);
        // Held by a starting or running session, it is not deleted.
        app.egresses.lock().unwrap().reserve(&id, false, "0123456789ab").unwrap();
        let path = format!("/api/egresses/{id}");
        let (status, _, refused) = request(&app, "DELETE", &path, ORIGIN, "", login).await;
        assert_eq!((status, &refused["sessions"]), (StatusCode::CONFLICT, &json!(["0123456789ab"])));
        app.egresses.lock().unwrap().release("0123456789ab");
        assert_eq!(request(&app, "DELETE", &path, ORIGIN, "", login).await.0, StatusCode::OK);
    }
}
