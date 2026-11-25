//! The sessions this computer is running: several at once, one per show.

use std::collections::HashMap;
use std::sync::Arc;

use protocol::discovery::AnnouncedSession;
use protocol::session::{AccessSettings, PROTOCOL_VERSION};
use showlogic::SessionLogic;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::discovery::{Announced, Responder};
use super::ports::PortRange;
use crate::caller_store::AccountVerifier;
use super::server::{Db, OnChange, RunningSession};

#[derive(Default)]
pub struct Sessions {
    /// Held across a start, so two starts of one show cannot both succeed.
    running: Mutex<HashMap<String, Arc<RunningSession>>>,
    /// `None`: sessions are not announced on the network (tests).
    discovery_port: Option<u16>,
    /// The running sessions as other apps get to see them.
    announced: Announced,
    /// Started with the first session and kept: it only answers while something is running.
    responder: Mutex<Option<Responder>>,
}

impl Sessions {
    /// Sessions other ShowFlow apps on the network can find.
    pub fn discoverable(port: u16) -> Self {
        Self { discovery_port: Some(port), ..Self::default() }
    }

    /// False if this computer cannot answer searches (for example: another ShowFlow already
    /// holds the port). Joining by address still works then.
    pub async fn is_discoverable(&self) -> bool {
        self.responder.lock().await.is_some()
    }

    /// Not being found is no reason to refuse to run a show, so a failure is only logged. The
    /// next start tries again.
    async fn answer_searches(&self) {
        let Some(port) = self.discovery_port else { return };
        let mut responder = self.responder.lock().await;
        if responder.is_none() {
            match Responder::start(port, self.announced.clone()).await {
                Ok(started) => *responder = Some(started),
                Err(error) => eprintln!("Sessions cannot be discovered: {error}"),
            }
        }
    }

    fn announce(&self, running: &HashMap<String, Arc<RunningSession>>) {
        *self.announced.lock().unwrap() = running
            .values()
            .map(|session| AnnouncedSession {
                session_id: session.session_id.clone(),
                show_name: session.show_name.clone(),
                port: session.http_port,
                protocol: PROTOCOL_VERSION,
            })
            .collect();
    }

    pub async fn start(
        &self,
        show_id: &str,
        show_name: String,
        settings: AccessSettings,
        db: Db,
        range: PortRange,
        on_change: OnChange,
    ) -> Result<Arc<RunningSession>, String> {
        let mut running = self.running.lock().await;
        if running.values().any(|session| session.show_id == show_id) {
            return Err("This show already has a running session".into());
        }
        let mut logic = SessionLogic::new(
            Uuid::new_v4().to_string(),
            show_id.to_owned(),
            showlogic::code::random_hex(64),
            settings,
        )
        .map_err(|e| e.to_string())?;
        logic.set_verifier(Arc::new(AccountVerifier { db: db.clone() }));
        let session = RunningSession::start(logic, show_name, db, range, on_change).await?;
        running.insert(session.session_id.clone(), session.clone());
        self.answer_searches().await;
        self.announce(&running);
        Ok(session)
    }

    pub async fn get(&self, session_id: &str) -> Result<Arc<RunningSession>, String> {
        self.running.lock().await.get(session_id).cloned().ok_or_else(|| "That session is not running".to_string())
    }

    /// Every running session, oldest show id first so lists are stable.
    pub async fn list(&self) -> Vec<Arc<RunningSession>> {
        let mut sessions: Vec<_> = self.running.lock().await.values().cloned().collect();
        sessions.sort_by(|a, b| a.show_id.cmp(&b.show_id));
        sessions
    }

    pub async fn stop(&self, session_id: &str) -> Result<(), String> {
        let mut running = self.running.lock().await;
        let session = running.remove(session_id).ok_or_else(|| "That session is not running".to_string())?;
        self.announce(&running);
        session.stop();
        Ok(())
    }
}
