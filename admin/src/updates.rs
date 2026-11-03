//! `/ws/updates`: tells the administrator's pages that what their lists of sessions show may have changed, so they read
//! them again then and only then: a word and nothing else, as the lists stay behind the same routes and checks. Never in
//! the demo, whose visitors have no lists.

use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};

use crate::api::{Shared, not_in_the_demo};
use crate::auth::LoginKey;

/// How long changes settle before they are told, so a burst is told once.
const SETTLES: Duration = Duration::from_millis(300);
/// How often the connection and its login are looked at.
const LOOK_EVERY: Duration = Duration::from_secs(20);
/// How long a write may wait: past it, the page is gone.
const WRITE_WITHIN: Duration = crate::deadline::WRITE;

pub async fn updates(State(app): State<Shared>, headers: HeaderMap, upgrade: WebSocketUpgrade) -> Response {
    let login = match app.auth.lock().unwrap().check_upgrade(&headers, &app.config.origin) {
        Ok(key) => key,
        Err(r) => return r.into_response(),
    };
    if app.config.demo.is_some() {
        return not_in_the_demo();
    }
    upgrade.max_message_size(1024).on_upgrade(move |socket| run(app, login, socket))
}

async fn run(app: Shared, login: LoginKey, mut socket: WebSocket) {
    let mut changes = crate::app::list_changes();
    // Whatever changed before this socket followed: the page read its list before opening it.
    if !sent(&mut socket, Message::Text("changed".into())).await {
        return;
    }
    let mut look = tokio::time::interval(LOOK_EVERY);
    loop {
        tokio::select! {
            changed = changes.changed() => {
                if changed.is_err() {
                    return;
                }
                tokio::time::sleep(SETTLES).await;
                changes.mark_unchanged();
                if !sent(&mut socket, Message::Text("changed".into())).await {
                    return;
                }
            }
            message = socket.recv() => match message {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
                Some(Ok(_)) => {}
            },
            _ = look.tick() => {
                // An open page is someone at it, as long as it answers; its login stays.
                if !app.auth.lock().unwrap().touch(&login) || !sent(&mut socket, Message::Ping(Vec::new().into())).await {
                    return;
                }
            }
        }
    }
}

async fn sent(socket: &mut WebSocket, message: Message) -> bool {
    matches!(tokio::time::timeout(WRITE_WITHIN, socket.send(message)).await, Ok(Ok(())))
}
