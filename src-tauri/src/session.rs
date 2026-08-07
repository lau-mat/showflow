use std::sync::{Arc, Mutex};
use serde::{Serialize, Deserialize};
use tauri::State;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;
use futures_util::{SinkExt, StreamExt}; // Add these imports
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio::sync::mpsc;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Session {
    pub session_id: String,
    pub show_id: i64,
    pub is_active: bool,
    pub current_cue: Option<i64>,
    pub joined_users: Vec<String>,
}

pub struct AppState {
    pub session: Mutex<Option<Session>>,
    pub shutdown_tx: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Client {
    pub client_id: String,
    pub client_name: Option<String>,
    pub client_role: Option<i64>
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum ClientMessage {
    JoinSession { user_name: String, role_id: Option<i64>}
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum ServerMessage {
    JoinSession { user_id: String, active_cue: Option<i64>  }
}

// remember to call `.manage(MyState::default())`
#[tauri::command]
pub async fn start_session(show_id: i64, state: State<'_, AppState>) -> Result<String, String> {
    let mut session_guard = state.session.lock().unwrap();
    if session_guard.is_some() {
        return Err("Session already running".into());
    }

    //create active session
    let session_id = uuid::Uuid::new_v4().to_string();
    let new_session = Session {
        session_id: session_id.clone(),
        show_id,
        is_active: true,
        current_cue: None,
        joined_users: Vec::new(),
    };
    *session_guard = Some(new_session);
    
    //setup gracefull shutdown channel
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    *state.shutdown_tx.lock().unwrap() = Some(tx);

    let clients: Arc<Mutex<Vec<Client>>> = Arc ::new(Mutex::new(Vec::new()));
    let server_session_id = session_id.clone();

    //spawn background websocket server
    tokio::spawn(async move {
       let listener = TcpListener::bind("0.0.0.0:23123").await.expect("Failed to bind WebSocket server");

       tokio::select! {
            _ = async {
                while let Ok((stream, _)) = listener.accept().await {
                    let clients_arc = Arc::clone(&clients);
                    let client_session_id = server_session_id.clone();

                    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

                    tokio::spawn(async move {
                        if let Ok(ws_stream) = accept_async(stream).await {
                            let client = Client {
                                client_id: uuid::Uuid::new_v4().to_string(),
                                client_name: None,
                                client_role: None
                            };

                            println!("Client {} connected to live session {}", client.client_id, client_session_id);

                            {
                                let mut guard = clients_arc.lock().unwrap();
                                guard.push(client.clone());
                            }

                            let (mut write, mut read) = ws_stream.split();
                            while let Some(message) = read.next().await {
                                match message {
                                    Ok(msg) => {
                                        if msg.is_text() {
                                            let text = msg.to_text().unwrap_or_default();
                                            println!("Received from client {}: {}", client.client_id, text);

                                            let reply = format!("Server acknowledgment: {}", text);
                                            if let Err(e) = write.send(Message::Text(reply)).await {
                                                println!("Failed to send message: {}", e);
                                                break;
                                            }
                                        } else if msg.is_close() {
                                            break;
                                        }
                                    }

                                    Err(e) => {
                                        println!("WebSocket read error: {}", e);
                                        break;
                                    }
                                }
                            }

                            {
                                let mut guard = clients_arc.lock().unwrap();
                                guard.retain(|c| c.client_id != client.client_id);
                            }

                            println!("client {} disconnected", client.client_id);
                        }
                    });
                }
            } => {},
            _ = rx => {
                // Graceful shutdown logic here
                println!("Shutting down WebSocket server...");
            }
       }
       Some(())
    });

    Ok(session_id)
}