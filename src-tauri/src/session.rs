use crate::{ShowDetails, db, fetch_show_details};

use std::sync::{Arc, Mutex};
use serde::{Serialize, Deserialize};
use tauri::State;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;
use futures_util::{SinkExt, StreamExt}; // Add these imports
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio::sync::mpsc;
use axum::{
    body::Body,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
    Router,
};
use rust_embed::RustEmbed;
use tauri::{AppHandle, WebviewUrl, WebviewWindowBuilder};
use qrcode::QrCode;
use qrcode::render::svg;
use local_ip_address::local_ip;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Session {
    pub session_id: String,
    pub show_id: i64,
    pub is_active: bool,
    pub current_cue: Option<i64>,
    pub joined_users: Vec<String>,
}

pub struct AppState {
    pub session: Arc<Mutex<Option<Session>>>,
    pub shutdown_tx: Mutex<Option<tokio::sync::broadcast::Sender<()>>>,
}

#[derive(Clone, Debug)]
pub struct Client {
    pub client_id: String,
    pub client_name: Option<String>,
    pub client_role: Option<i64>,
    pub sender: mpsc::UnboundedSender<Message>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ClientMessage {
    JoinSession { user_name: String, role_id: Option<i64>},
    GetShowData,
    GetSessionData,
    NextCue
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ServerMessage {
    JoinSession { user_id: String, active_cue: Option<i64>  },
    GetShowData { data: ShowDetails },
    GetSessionData { session: Session },
    Error { message: String }
}

async fn send_error<S>(writer: &mut S, error_message: &str)
where
    S: SinkExt<Message> + Unpin,
    S::Error: std::fmt::Display, // Matches the bound required by send_json
{
    let error_response = ServerMessage::Error {
        message: error_message.to_string(),
    };
    
    match serde_json::to_string(&error_response) {
        Ok(json_text) => {
            if let Err(e) = writer.send(Message::Text(json_text)).await {
                eprintln!("Failed to send error message: {}", e);
            }
        }
        Err(e) => {
            eprintln!("Failed to serialize error response: {}", e);
        }
    }

    eprintln!("{}", error_message);
}

async fn send_json<S, T>(writer: &mut S, payload: &T)
where
    S: SinkExt<Message> + Unpin,
    S::Error: std::fmt::Display,
    T: Serialize,
{
    match serde_json::to_string(payload) {
        Ok(json_text) => {
            if let Err(e) = writer.send(Message::Text(json_text)).await {
                let error_msg = format!("Failed to send WebSocket message: {}", e);
                send_error(writer, &error_msg).await;
            }
        }
        Err(e) => {
            let error_msg = format!("Failed to serialize response: {}", e);
            send_error(writer, &error_msg).await;
        }
    }
}

async fn broadcast_json<T>(clients: &Arc<Mutex<Vec<Client>>>, payload: &T)
where
    T: Serialize,
{
    match serde_json::to_string(payload) {
        Ok(json_text) => {
            let clients_guard = clients.lock().unwrap();
            
            println!("📢 Broadcasting to {} client(s): {}", clients_guard.len(), json_text);

            if clients_guard.is_empty() {
                println!("⚠️ Broadcast warning: Connected clients list is empty!");
                return;
            }

            for client in clients_guard.iter() {
                if let Err(e) = client.sender.send(Message::Text(json_text.clone())) {
                    println!("❌ Failed to send to client channel {}: {}", client.client_id, e);
                } else {
                    println!("✅ Sent broadcast packet to channel for client {}", client.client_id);
                }
            }
        }
        Err(e) => {
            // If Serde fails to serialize ServerMessage, it will print here!
            println!("❌ Serialization failed in broadcast_json: {}", e);
        }
    }
}

#[derive(RustEmbed, Clone)]
#[folder = "../client_source/"]
struct Asset;

async fn static_handler(uri: Uri) -> impl IntoResponse {
    let mut path = uri.path().trim_start_matches('/').to_string();

    // Default to index.html for root requests
    if path.is_empty() {
        path = "index.html".to_string();
    }

    match Asset::get(&path) {
        Some(content) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            Response::builder()
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(Body::from(content.data))
                .unwrap()
        }
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("404 Not Found"))
            .unwrap(),
    }
}

// remember to call `.manage(MyState::default())`
#[tauri::command]
pub async fn start_session(show_id: i64, state: State<'_, AppState>, db_state: State<'_, db::DbState>, app_handle: AppHandle,) -> Result<String, String> {
    let mut session_guard = state.session.lock().unwrap();
    if session_guard.is_some() {
        return Err("Session already running".into());
    }

    //create active session
    let session_id = uuid::Uuid::new_v4().to_string();
    *session_guard = Some(Session {
        session_id: session_id.clone(),
        show_id,
        is_active: true,
        current_cue: None,
        joined_users: Vec::new(),
    });
    
    //setup gracefull shutdown channel
    let (tx, _rx) = tokio::sync::broadcast::channel::<()>(1);
    let mut rx_http = tx.subscribe();
    let mut rx_ws = tx.subscribe();

    *state.shutdown_tx.lock().unwrap() = Some(tx);

    let server_session_id = session_id.clone();
    let db_arc = Arc::clone(&db_state.0);
    let session_arc = Arc::clone(&state.session);
    let clients: Arc<Mutex<Vec<Client>>> = Arc::new(Mutex::new(Vec::new()));

    //spawn background websocket server
    tokio::spawn(async move {
       let listener = TcpListener::bind("0.0.0.0:23123").await.expect("Failed to bind WebSocket server");

       tokio::select! {
            _ = async {
                while let Ok((stream, _)) = listener.accept().await {
                    let clients_arc = Arc::clone(&clients);
                    let client_session_id = server_session_id.clone();
                    let db = Arc::clone(&db_arc);
                    let session_for_client = Arc::clone(&session_arc);

                    tokio::spawn(async move {
                        if let Ok(ws_stream) = accept_async(stream).await {
                            let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

                            let client = Client {
                                client_id: uuid::Uuid::new_v4().to_string(),
                                client_name: None,
                                client_role: None,
                                sender: tx,
                            };

                            println!("Client {} connected to live session {}", client.client_id, client_session_id);

                            {
                                let mut guard = clients_arc.lock().unwrap();
                                guard.push(client.clone());
                            }

                            let (mut write, mut read) = ws_stream.split();

                            loop {
                                tokio::select! {
                                    // Forward broadcast/outgoing messages from channel to WebSocket
                                    Some(outgoing) = rx.recv() => {
                                        if write.send(outgoing).await.is_err() {
                                            break;
                                        }
                                    }
                                    // Handle incoming WebSocket messages
                                    msg = read.next() => {
                                        let text = match msg {
                                            Some(Ok(Message::Text(t))) => t,
                                            _ => break,
                                        };
                                        match serde_json::from_str::<ClientMessage>(&text) {
                                            Ok(ClientMessage::JoinSession { user_name, role_id }) => {
                                                println!("User {} joined with role {:?}", user_name, role_id);

                                                let response = ServerMessage::JoinSession {
                                                    user_id: client.client_id.clone(),
                                                    active_cue: None
                                                };

                                                send_json(&mut write, &response).await;
                                            }

                                            Ok(ClientMessage::GetShowData) => {
                                                match fetch_show_details(&db, show_id) {
                                                    Ok(show_details) => {
                                                        let reponse = ServerMessage::GetShowData { data: show_details };
                                                        send_json(&mut write, &reponse).await;
                                                    }
                                                    Err(err) => {
                                                        let error_msg = format!("Failed to fetch show details: {}", err);
                                                        send_error(&mut write, &error_msg).await;
                                                    }
                                                }
                                            }

                                            Ok(ClientMessage::GetSessionData) => {
                                                let response = {
                                                    let session_guard = session_for_client.lock().unwrap();
                                                    if let Some(session) = &*session_guard {
                                                        Ok(ServerMessage::GetSessionData { session: session.clone() })
                                                    } else {
                                                        Err("No active session found".to_string())
                                                    }
                                                };

                                                match response {
                                                    Ok(resp) => send_json(&mut write, &resp).await,
                                                    Err(err) => send_error(&mut write, &err).await,
                                                }
                                            }

                                            Ok(ClientMessage::NextCue) => {
                                                let response = {
                                                    let mut session_guard = session_for_client.lock().unwrap();
                                                    if let Some(session) = &mut *session_guard {
                                                        if let Some(current_cue) = session.current_cue {
                                                            session.current_cue = Some(current_cue + 1);
                                                        } else {
                                                            session.current_cue = Some(1);
                                                        }
                                                        Ok(ServerMessage::GetSessionData { session: session.clone() })
                                                    } else {
                                                        Err("No active session found".to_string())
                                                    }
                                                };
                                                println!("Client {} requested next cue", client.client_id);
                                                match response {
                                                    Ok(resp) => broadcast_json(&clients_arc, &resp).await,
                                                    Err(err) => send_error(&mut write, &err).await,
                                                }
                                            }

                                            Err(err) => {
                                                let error_msg = format!("Invalid client message received: {}", err);
                                                send_error(&mut write, &error_msg).await;
                                            }
                                        }
                                    } // msg arm
                                } // select!
                            } // loop

                            {
                                let mut guard = clients_arc.lock().unwrap();
                                guard.retain(|c| c.client_id != client.client_id);
                            }

                            println!("client {} disconnected", client.client_id);
                        }
                    });
                }
            } => {},
            _ = rx_ws.recv() => {
                // Graceful shutdown logic here
                println!("Shutting down WebSocket server...");
            }
       }
       Some(())
    });

    //start webserver
    tokio::spawn(async move {
        let app = Router::new().fallback(static_handler);
        if let Ok(listener) = tokio::net::TcpListener::bind("0.0.0.0:8080").await {
            println!("Embedded Web Server running at http://0.0.0.0:8080");
            
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = rx_http.recv().await;
                    println!("Shutting down HTTP server...");
                })
                .await
                .ok();
        }
    });

    // Open master window
    WebviewWindowBuilder::new(
        &app_handle,
        "session-manager",
        WebviewUrl::App("session.html".into())
    )
    .title("Session Management")
    .inner_size(1024.0, 768.0)
    .devtools(true)
    .build()
    .map_err(|e| e.to_string())?;

    Ok(session_id)
}

#[tauri::command]
pub fn generate_server_qr() -> Result<String, String> {
    let my_local_ip = local_ip().map_err(|e| e.to_string())?;
    let server_url = format!("http://{}:8080", my_local_ip);

    // 2. Encode the URL into a QR code SVG string
    let code = QrCode::new(server_url.as_bytes()).map_err(|e| e.to_string())?;
    let svg_string = code.render::<svg::Color>()
        .min_dimensions(200, 200)
        .build();

    Ok(svg_string)
}