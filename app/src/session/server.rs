//! The transport of one running session: a websocket for the live protocol and a small web server
//! for the crew and caller pages. All decisions are made by `showlogic`; this only moves messages.
//! No Tauri here, so it can be tested with real sockets.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use protocol::session::{ClientMessage, ServerMessage, PROTOCOL_VERSION};
use rusqlite::Connection;
use rust_embed::RustEmbed;
use serde::Serialize;
use showlogic::{Effect, SessionLogic};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request as WsRequest, Response as WsResponse};
use tokio_tungstenite::tungstenite::protocol::{Message, WebSocketConfig};
use uuid::Uuid;

use super::ports::{bind_two, PortRange};
use crate::store;

/// Larger than any message a client has reason to send.
const MAX_MESSAGE_BYTES: usize = 64 * 1024;

pub type Db = Arc<Mutex<Connection>>;
type Clients = Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Message>>>>;
/// Called whenever something the host's panel shows has changed.
pub type OnChange = Arc<dyn Fn(&str) + Send + Sync>;

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

#[derive(Serialize, Clone)]
struct SessionInfoResponse {
    protocol: u32,
    session_id: String,
    show_name: String,
    ws_port: u16,
}

pub struct RunningSession {
    pub session_id: String,
    pub show_id: String,
    pub show_name: String,
    pub http_port: u16,
    pub ws_port: u16,
    logic: Mutex<SessionLogic>,
    clients: Clients,
    db: Db,
    on_change: OnChange,
    shutdown: broadcast::Sender<()>,
}

impl RunningSession {
    /// Binds both ports (so success means the session is really reachable) and starts serving.
    pub async fn start(
        logic: SessionLogic,
        show_name: String,
        db: Db,
        range: PortRange,
        on_change: OnChange,
    ) -> Result<Arc<Self>, String> {
        let (http, websocket) = bind_two(range).await?;
        let port = |listener: &TcpListener| listener.local_addr().map(|a| a.port()).map_err(|e| e.to_string());
        let (shutdown, _) = broadcast::channel(1);
        let session = Arc::new(Self {
            session_id: logic.session_id().to_owned(),
            show_id: logic.show_id().to_owned(),
            show_name,
            http_port: port(&http)?,
            ws_port: port(&websocket)?,
            logic: Mutex::new(logic),
            clients: Clients::default(),
            db,
            on_change,
            shutdown,
        });
        tokio::spawn(accept_websockets(session.clone(), websocket, session.shutdown.subscribe()));
        tokio::spawn(serve_pages(session.clone(), http, session.shutdown.subscribe()));
        Ok(session)
    }

    /// Ends the session: every connection is closed and both ports are freed.
    pub fn stop(&self) {
        let _ = self.shutdown.send(());
        self.clients.lock().unwrap().clear();
    }

    /// Reads something from the show logic.
    pub fn inspect<T>(&self, read: impl FnOnce(&SessionLogic) -> T) -> T {
        read(&self.logic.lock().unwrap())
    }

    /// Runs one step of the show logic and carries out what it asks for. The lock is held while
    /// messages are queued, so everyone receives them in the order things happened.
    pub fn act<T>(&self, step: impl FnOnce(&mut SessionLogic) -> (Vec<Effect>, T)) -> T {
        let (changed, result) = {
            let mut logic = self.logic.lock().unwrap();
            let (effects, result) = step(&mut logic);
            let mut changed = false;
            for effect in effects {
                changed |= self.carry_out(&logic, effect);
            }
            (changed, result)
        };
        // Outside the lock: the listener may well come back and ask about the session.
        if changed {
            (self.on_change)(&self.session_id);
        }
        result
    }

    pub fn apply(&self, step: impl FnOnce(&mut SessionLogic) -> Vec<Effect>) {
        self.act(|logic| (step(logic), ()));
    }

    fn send(&self, to: &str, message: &ServerMessage) {
        if let (Some(sender), Ok(text)) = (self.clients.lock().unwrap().get(to), serde_json::to_string(message)) {
            let _ = sender.send(Message::Text(text));
        }
    }

    /// Returns true if the host panel should refresh.
    fn carry_out(&self, logic: &SessionLogic, effect: Effect) -> bool {
        match effect {
            Effect::Send(to, message) => self.send(&to, &message),
            Effect::Broadcast(message) => {
                if let Ok(text) = serde_json::to_string(&message) {
                    // Only to devices that are in: a protected session tells the others nothing.
                    for (id, sender) in self.clients.lock().unwrap().iter() {
                        if logic.is_admitted(id) {
                            let _ = sender.send(Message::Text(text.clone()));
                        }
                    }
                }
            }
            Effect::SendShowData(to) => {
                let details = self
                    .db
                    .lock()
                    .map_err(|e| e.to_string())
                    .and_then(|conn| store::fetch_show_details(&conn, &self.show_id));
                match details {
                    Ok(data) => self.send(&to, &ServerMessage::GetShowData { data }),
                    Err(error) => self.send(&to, &ServerMessage::Error { message: format!("Failed to fetch show details: {error}") }),
                }
            }
            // Dropping the sender lets the connection flush what is queued, then close.
            Effect::Close(id) => {
                self.clients.lock().unwrap().remove(&id);
            }
            Effect::HostChanged => return true,
        }
        false
    }
}

/// Browsers send an `Origin` with every websocket. Only pages served by this machine's session (the
/// same host name, whatever the port) or by the ShowFlow app itself may connect, so a random web
/// page someone has open on the venue network cannot drive the show. Non-browser clients send none.
fn origin_allowed(origin: Option<&str>, host: Option<&str>) -> bool {
    const APP_ORIGINS: [&str; 3] = ["tauri://localhost", "http://tauri.localhost", "https://tauri.localhost"];
    fn host_name(authority: &str) -> &str {
        if let Some(rest) = authority.strip_prefix('[') {
            return rest.split(']').next().unwrap_or(rest);
        }
        authority.split(':').next().unwrap_or(authority)
    }
    let Some(origin) = origin else { return true };
    // `tauri dev` serves the app's pages from Vite; release builds never trust that origin.
    let dev_app = cfg!(debug_assertions) && origin == "http://127.0.0.1:1420";
    if APP_ORIGINS.contains(&origin) || dev_app {
        return true;
    }
    match (origin.split_once("://"), host) {
        (Some((_, authority)), Some(host)) => host_name(authority).eq_ignore_ascii_case(host_name(host)),
        _ => false,
    }
}

async fn accept_websockets(session: Arc<RunningSession>, listener: TcpListener, mut shutdown: broadcast::Receiver<()>) {
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    tokio::spawn(handle_connection(session.clone(), stream, peer, session.shutdown.subscribe()));
                }
                Err(error) => eprintln!("Could not accept a connection: {error}"),
            },
            _ = shutdown.recv() => break,
        }
    }
}

async fn handle_connection(session: Arc<RunningSession>, stream: TcpStream, peer: SocketAddr, mut shutdown: broadcast::Receiver<()>) {
    #[allow(clippy::result_large_err)]
    let check_origin = |request: &WsRequest, response: WsResponse| -> Result<WsResponse, ErrorResponse> {
        let header = |name: &str| request.headers().get(name).and_then(|v| v.to_str().ok());
        if origin_allowed(header("origin"), header("host")) {
            Ok(response)
        } else {
            let mut refused = ErrorResponse::new(Some("This page may not connect to the session".into()));
            *refused.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::FORBIDDEN;
            Err(refused)
        }
    };
    let config = WebSocketConfig { max_message_size: Some(MAX_MESSAGE_BYTES), max_frame_size: Some(MAX_MESSAGE_BYTES), ..Default::default() };
    let Ok(socket) = tokio_tungstenite::accept_hdr_async_with_config(stream, check_origin, Some(config)).await else {
        return;
    };

    let id = Uuid::new_v4().to_string();
    let (sender, mut outgoing) = mpsc::unbounded_channel::<Message>();
    session.clients.lock().unwrap().insert(id.clone(), sender);
    session.apply(|logic| logic.connect(&id, &peer.ip().to_string()));

    let (mut write, mut read) = socket.split();
    loop {
        tokio::select! {
            queued = outgoing.recv() => match queued {
                Some(message) => if write.send(message).await.is_err() { break },
                None => break,
            },
            received = read.next() => match received {
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<ClientMessage>(&text) {
                    Ok(message) => session.apply(|logic| logic.handle(&id, message, now())),
                    Err(error) => session.send(&id, &ServerMessage::Error { message: format!("Invalid client message received: {error}") }),
                },
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                _ => break,
            },
            _ = shutdown.recv() => break,
        }
    }
    let _ = write.close().await;
    session.clients.lock().unwrap().remove(&id);
    session.apply(|logic| logic.disconnect(&id));
}

#[derive(RustEmbed, Clone)]
#[folder = "../dist/session/"]
struct Asset;

async fn static_handler(uri: Uri) -> impl IntoResponse {
    let mut path = uri.path().trim_start_matches('/').to_string();
    if path.is_empty() {
        path = "client.html".to_string();
    }
    match Asset::get(&path) {
        Some(content) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            Response::builder()
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(Body::from(content.data))
                .unwrap()
        }
        None => Response::builder().status(StatusCode::NOT_FOUND).body(Body::from("404 Not Found")).unwrap(),
    }
}

/// Where the websocket is. Public and secret-free, and readable cross-origin so the ShowFlow app
/// can ask a host it is about to join.
async fn session_info(State(info): State<SessionInfoResponse>) -> impl IntoResponse {
    ([(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"), (header::CACHE_CONTROL, "no-store")], Json(info))
}

async fn serve_pages(session: Arc<RunningSession>, listener: TcpListener, mut shutdown: broadcast::Receiver<()>) {
    let info = SessionInfoResponse {
        protocol: PROTOCOL_VERSION,
        session_id: session.session_id.clone(),
        show_name: session.show_name.clone(),
        ws_port: session.ws_port,
    };
    let app = Router::new().route("/session-info", get(session_info)).fallback(static_handler).with_state(info);
    let served = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown.recv().await;
        })
        .await;
    if let Err(error) = served {
        eprintln!("The session web server stopped: {error}");
    }
}

#[cfg(test)]
mod tests;
