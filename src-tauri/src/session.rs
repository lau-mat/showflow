use std::sync::Mutex;
use serde::{Serialize, Deserialize};
use tauri::State;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;

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

// remember to call `.manage(MyState::default())`
#[tauri::command]
pub async fn start_session(show_id: String, state: State<'_, AppState>) -> Result<String, String> {
    let mut session_guard = state.session.lock().unwrap();
    if session_guard.is_some() {
        return Err("Session already running".into());
    }

    //create active session
    let session_id = uuid::Uuid::new_v4().to_string();
    let new_session = Session {
        session_id: session_id.clone(),
        show_id: show_id.parse::<i64>().map_err(|e| e.to_string())?,
        is_active: true,
        current_cue: None,
        joined_users: Vec::new(),
    };
    *session_guard = Some(new_session);
    
    //setup gracefull shutdown channel
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    *state.shutdown_tx.lock().unwrap() = Some(tx);

    //spawn background websocket server
    tokio::spawn(async move {
       let listener = TcpListener::bind("0.0.0.0:8080").await.expect("Failed to bind WebSocket server");

       tokio::select! {
            _ = async {
                while let Ok((stream, _)) = listener.accept().await {
                    tokio::spawn(async move {
                        if let Ok(ws_stream) = accept_async(stream).await {
                            // Handle the WebSocket connection here
                            // For example, you can read messages from the client and respond accordingly
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