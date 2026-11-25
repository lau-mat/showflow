use std::collections::HashMap;
use std::sync::Arc;

use protocol::session::{
    AccessKind, AccessMethod, AccessPolicy, AccessSettings, AccessState, CallerInfo, Claim, ClientMessage,
    JoinedUser, ServerMessage, SessionSnapshot, PROTOCOL_VERSION,
};
use uuid::Uuid;

use crate::code::{self, CodeError};
use crate::limiter::{CodeLimiter, MAX_TRIES_PER_CONNECTION};

const MAX_NAME_CHARS: usize = 40;
const KINDS: [AccessKind; 2] = [AccessKind::View, AccessKind::Call];
pub type ConnectionId = String;

/// What the host has to do after feeding the logic something.
#[derive(Debug, PartialEq)]
pub enum Effect {
    Send(ConnectionId, ServerMessage),
    /// To every connection that has been let in (see `SessionLogic::is_admitted`), and no other.
    Broadcast(ServerMessage),
    /// Send this connection the show's current content (the logic does not know the script).
    SendShowData(ConnectionId),
    /// Drop the connection, after whatever was queued for it.
    Close(ConnectionId),
    /// Something the host's session panel shows changed.
    HostChanged,
}

/// Verifies account-based claims: a pass signed by the cloud plus the device's proof. The host
/// provides it; without one, account claims are refused.
pub trait PassVerifier: Send + Sync {
    /// Returns who the claimant is, or why the claim is refused (shown to them).
    fn verify(&self, pass: &str, proof: &str, context: &PassContext<'_>) -> Result<VerifiedUser, String>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct VerifiedUser {
    pub user_id: String,
    pub name: String,
}

pub struct PassContext<'a> {
    pub show_id: &'a str,
    pub session_id: &'a str,
    pub nonce: &'a str,
    pub now: f64,
    /// Joining takes any member of the show; calling takes one marked as caller.
    pub kind: AccessKind,
}

#[derive(Default)]
struct Connection {
    address: String,
    nonce: String,
    /// Set once the device joined as crew.
    name: Option<String>,
    role: Option<String>,
    /// Per gate (indexed by `AccessKind`): the grant this connection acts under.
    grants: [Option<String>; 2],
    /// Per gate: the name of a request waiting for the host.
    pending: [Option<String>; 2],
    wrong_codes: u32,
}

#[derive(Clone)]
struct Grant {
    id: String,
    name: String,
    method: AccessMethod,
    token: String,
    /// The code in force when it was granted; changing the code ends code grants.
    code_generation: u64,
    /// The account behind an account grant, so losing the right ends it.
    user_id: Option<String>,
}

/// One guarded thing: who may pass, and who did. Joining and calling follow the same rules.
#[derive(Default)]
struct Gate {
    policy: AccessPolicy,
    code: Option<String>,
    code_generation: u64,
    limiter: CodeLimiter,
    /// In the order they were granted.
    grants: Vec<Grant>,
}

impl Gate {
    fn new(policy: AccessPolicy, code: Option<String>) -> Result<Self, CodeError> {
        if let Some(code) = &code {
            code::validate(code)?;
        }
        let policy = policy.normalized();
        let code = code.or_else(|| policy.code.then(|| code::generate(code::DEFAULT_GENERATED_CHARS)));
        Ok(Self { policy, code, ..Self::default() })
    }

    fn still_valid(&self, grant: &Grant) -> bool {
        self.policy.allows(grant.method)
            && (grant.method != AccessMethod::Code || grant.code_generation == self.code_generation)
    }
}

#[derive(serde::Serialize, Debug, PartialEq)]
pub struct HostPerson {
    pub client_id: String,
    pub name: Option<String>,
    pub role_id: Option<String>,
    /// Let in: it can see the show.
    pub admitted: bool,
    pub is_caller: bool,
    /// The host computer's own caller window: not something to remove.
    pub is_host: bool,
}

#[derive(serde::Serialize, Debug, PartialEq)]
pub struct HostCaller {
    pub grant_id: String,
    pub name: String,
    pub method: AccessMethod,
    pub online: bool,
}

#[derive(serde::Serialize, Debug, PartialEq)]
pub struct HostRequest {
    pub client_id: String,
    pub name: String,
    pub kind: AccessKind,
}

/// Everything the host's panel shows. Contains the codes, so it must never go over the socket.
#[derive(serde::Serialize, Debug, PartialEq)]
pub struct HostView {
    pub settings: AccessSettings,
    /// Seconds left of a pause on code entry caused by many wrong tries.
    pub view_paused_seconds: Option<u64>,
    pub call_paused_seconds: Option<u64>,
    pub current_cue: Option<i64>,
    pub people: Vec<HostPerson>,
    pub callers: Vec<HostCaller>,
    pub requests: Vec<HostRequest>,
}

pub struct SessionLogic {
    session_id: String,
    show_id: String,
    host_token: String,
    view: Gate,
    call: Gate,
    current_cue: Option<i64>,
    show_started_at: Option<f64>,
    cue_started_at: Option<f64>,
    connections: HashMap<ConnectionId, Connection>,
    /// Connection order, so lists come out the same every time.
    order: Vec<ConnectionId>,
    /// Everyone who joined as crew, kept after they disconnect so a reload can restore them.
    joined: Vec<(ConnectionId, String, Option<String>)>,
    verifier: Option<Arc<dyn PassVerifier>>,
}

fn clean_name(name: &str) -> String {
    let cleaned: String = name.trim().chars().filter(|c| !c.is_control()).take(MAX_NAME_CHARS).collect();
    if cleaned.is_empty() { "Guest".to_owned() } else { cleaned }
}

fn secret() -> String {
    code::random_hex(64)
}

fn status(kind: AccessKind, connection: &str, state: AccessState, reason: Option<&str>, resume_token: Option<String>) -> Effect {
    let reason = reason.map(str::to_owned);
    let message = match kind {
        AccessKind::View => ServerMessage::ViewStatus { state, reason, resume_token },
        AccessKind::Call => ServerMessage::CallerStatus { state, reason, resume_token },
    };
    Effect::Send(connection.to_owned(), message)
}

fn denied(kind: AccessKind, connection: &str, reason: &str) -> Vec<Effect> {
    vec![status(kind, connection, AccessState::Denied, Some(reason), None)]
}

impl SessionLogic {
    /// `host_token` is the secret the host's own caller window uses. With code access enabled and
    /// no code given, one is generated.
    pub fn new(session_id: String, show_id: String, host_token: String, settings: AccessSettings) -> Result<Self, CodeError> {
        Ok(Self {
            session_id,
            show_id,
            host_token,
            view: Gate::new(settings.view, settings.view_code)?,
            call: Gate::new(settings.caller, settings.caller_code)?,
            current_cue: None,
            show_started_at: None,
            cue_started_at: None,
            connections: HashMap::new(),
            order: Vec::new(),
            joined: Vec::new(),
            verifier: None,
        })
    }

    pub fn set_verifier(&mut self, verifier: Arc<dyn PassVerifier>) {
        self.verifier = Some(verifier);
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn show_id(&self) -> &str {
        &self.show_id
    }

    pub fn host_token(&self) -> &str {
        &self.host_token
    }

    /// The settings in force, codes included.
    pub fn settings(&self) -> AccessSettings {
        AccessSettings {
            caller: self.call.policy,
            caller_code: self.call.code.clone(),
            view: self.view.policy,
            view_code: self.view.code.clone(),
        }
    }

    pub fn code(&self, kind: AccessKind) -> Option<&str> {
        self.gate(kind).code.as_deref()
    }

    fn gate(&self, kind: AccessKind) -> &Gate {
        match kind {
            AccessKind::View => &self.view,
            AccessKind::Call => &self.call,
        }
    }

    fn gate_mut(&mut self, kind: AccessKind) -> &mut Gate {
        match kind {
            AccessKind::View => &mut self.view,
            AccessKind::Call => &mut self.call,
        }
    }

    /// May this connection see the show? Everyone while joining is open; otherwise those who were
    /// let in, and callers (whose token or the host's secret got them there).
    pub fn is_admitted(&self, id: &str) -> bool {
        self.connections
            .get(id)
            .is_some_and(|c| self.view.policy.open || c.grants.iter().any(Option::is_some))
    }

    // -----------------------------------------------------------------------------------------
    // Connections
    // -----------------------------------------------------------------------------------------

    /// A device connected from `address` (used to slow down code guessing).
    pub fn connect(&mut self, id: &str, address: &str) -> Vec<Effect> {
        let nonce = code::random_hex(32);
        self.connections.insert(id.to_owned(), Connection { address: address.to_owned(), nonce: nonce.clone(), ..Connection::default() });
        self.order.push(id.to_owned());
        let mut effects = vec![Effect::Send(id.to_owned(), ServerMessage::Welcome {
            protocol: PROTOCOL_VERSION,
            session_id: self.session_id.clone(),
            show_id: self.show_id.clone(),
            nonce,
            policy: self.call.policy,
            view_policy: self.view.policy,
        })];
        if self.is_admitted(id) {
            effects.extend(self.show_to(id));
        }
        effects.push(Effect::HostChanged);
        effects
    }

    /// What a device that just got in needs to draw the show.
    fn show_to(&self, id: &str) -> [Effect; 3] {
        [
            Effect::SendShowData(id.to_owned()),
            Effect::Send(id.to_owned(), ServerMessage::GetSessionData { session: self.snapshot() }),
            Effect::Send(id.to_owned(), ServerMessage::Callers { list: self.callers() }),
        ]
    }

    /// Runs one change, then tells whoever got in or was put out because of it. Everything that
    /// can change who is admitted goes through here, so no path can forget to.
    fn tracking(&mut self, lost_reason: &str, change: impl FnOnce(&mut Self) -> Vec<Effect>) -> Vec<Effect> {
        let before: Vec<ConnectionId> = self.order.iter().filter(|id| self.is_admitted(id)).cloned().collect();
        let mut effects = change(self);
        for id in self.order.clone() {
            let (was, is) = (before.contains(&id), self.is_admitted(&id));
            if is && !was {
                let Some(connection) = self.connections.get_mut(&id) else { continue };
                connection.pending[AccessKind::View as usize] = None;
                // A grant of its own has already been announced, with its token.
                if connection.grants[AccessKind::View as usize].is_none() {
                    effects.push(status(AccessKind::View, &id, AccessState::Granted, None, None));
                }
                effects.extend(self.show_to(&id));
            } else if was && !is {
                effects.push(status(AccessKind::View, &id, AccessState::Revoked, Some(lost_reason), None));
            }
        }
        effects
    }

    pub fn disconnect(&mut self, id: &str) -> Vec<Effect> {
        let Some(connection) = self.connections.remove(id) else { return Vec::new() };
        self.order.retain(|other| other != id);
        let mut effects = vec![Effect::HostChanged];
        if connection.grants[AccessKind::Call as usize].is_some() {
            effects.push(self.callers_message());
        }
        effects
    }

    pub fn handle(&mut self, id: &str, message: ClientMessage, now: f64) -> Vec<Effect> {
        if !self.connections.contains_key(id) {
            return Vec::new();
        }
        self.tracking("You left the session", |logic| logic.dispatch(id, message, now))
    }

    fn dispatch(&mut self, id: &str, message: ClientMessage, now: f64) -> Vec<Effect> {
        let admitted = self.is_admitted(id);
        match message {
            ClientMessage::RequestView { claim } => self.claim(id, AccessKind::View, claim, now),
            ClientMessage::ClaimCaller { claim } => self.claim(id, AccessKind::Call, claim, now),
            ClientMessage::ReleaseCaller => self.release(id),
            // Nothing about the show goes to, or comes from, a device that is not in.
            ClientMessage::JoinSession { user_name, role_id } if admitted => self.join(id, &user_name, role_id),
            ClientMessage::GetShowData if admitted => vec![Effect::SendShowData(id.to_owned())],
            ClientMessage::GetSessionData if admitted => vec![
                Effect::Send(id.to_owned(), ServerMessage::GetSessionData { session: self.snapshot() }),
                Effect::Send(id.to_owned(), ServerMessage::Callers { list: self.callers() }),
            ],
            ClientMessage::JoinSession { .. } | ClientMessage::GetShowData | ClientMessage::GetSessionData => Vec::new(),
            ClientMessage::NextCue { from_cue } => self.step(id, from_cue, now, true),
            ClientMessage::PreviousCue { from_cue } => self.step(id, from_cue, now, false),
            ClientMessage::JumpToCue { cue_number } => match self.caller_name(id) {
                None => self.refuse_control(id),
                Some(by) if cue_number >= 1 => vec![self.change_cue(cue_number, false, by, now), Effect::HostChanged],
                Some(_) => Vec::new(),
            },
        }
    }

    fn join(&mut self, id: &str, name: &str, role_id: Option<String>) -> Vec<Effect> {
        let name = clean_name(name);
        if let Some(connection) = self.connections.get_mut(id) {
            connection.name = Some(name.clone());
            connection.role = role_id.clone();
        }
        match self.joined.iter_mut().find(|(other, _, _)| other == id) {
            Some(entry) => *entry = (id.to_owned(), name.clone(), role_id.clone()),
            None => self.joined.push((id.to_owned(), name.clone(), role_id.clone())),
        }
        vec![
            Effect::Broadcast(ServerMessage::JoinSession {
                client_id: id.to_owned(),
                client_name: name,
                client_role: role_id.unwrap_or_default(),
            }),
            Effect::HostChanged,
        ]
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            session_id: self.session_id.clone(),
            show_id: self.show_id.clone(),
            is_active: true,
            current_cue: self.current_cue,
            show_started_at: self.show_started_at,
            cue_started_at: self.cue_started_at,
            joined_users: self
                .joined
                .iter()
                .map(|(id, name, role)| JoinedUser {
                    client_id: id.clone(),
                    client_name: Some(name.clone()),
                    client_role: role.clone(),
                    is_caller: self.connections.get(id).is_some_and(|c| c.grants[AccessKind::Call as usize].is_some()),
                })
                .collect(),
        }
    }

    // -----------------------------------------------------------------------------------------
    // Getting through a gate
    // -----------------------------------------------------------------------------------------

    fn claim(&mut self, id: &str, kind: AccessKind, claim: Claim, now: f64) -> Vec<Effect> {
        if let Some(grant) = self.grant_of(id, kind) {
            // Already through: say so again rather than creating a second grant.
            return vec![status(kind, id, AccessState::Granted, None, Some(grant.token))];
        }
        if kind == AccessKind::View && self.view.policy.open {
            return vec![status(kind, id, AccessState::Granted, None, None)];
        }
        // Calling does not open the door: a protected session cannot be entered through the
        // caller role. A token or the host secret proves the device was let through before.
        if kind == AccessKind::Call && !self.is_admitted(id) && !matches!(claim, Claim::Resume { .. } | Claim::Host { .. }) {
            return denied(kind, id, "Join the session first");
        }
        let policy = self.gate(kind).policy;
        match claim {
            Claim::Open { name } => {
                if !policy.allows(AccessMethod::Open) {
                    return denied(kind, id, "Open access is off");
                }
                self.grant(id, kind, AccessMethod::Open, &name, None)
            }
            Claim::Code { name, code } => self.claim_with_code(id, kind, &name, &code, now),
            Claim::Request { name } => {
                if !policy.allows(AccessMethod::Request) {
                    return denied(kind, id, "Requests are off");
                }
                if let Some(connection) = self.connections.get_mut(id) {
                    connection.pending[kind as usize] = Some(clean_name(&name));
                }
                vec![status(kind, id, AccessState::Pending, None, None), Effect::HostChanged]
            }
            Claim::Account { pass, proof } => {
                if !policy.allows(AccessMethod::Account) {
                    return denied(kind, id, "Account access is off");
                }
                let Some(verifier) = self.verifier.clone() else {
                    return denied(kind, id, "This host cannot check account passes yet");
                };
                let nonce = self.connections.get(id).map(|c| c.nonce.clone()).unwrap_or_default();
                let context = PassContext { show_id: &self.show_id, session_id: &self.session_id, nonce: &nonce, now, kind };
                match verifier.verify(&pass, &proof, &context) {
                    Ok(user) => self.grant(id, kind, AccessMethod::Account, &user.name, Some(user.user_id)),
                    Err(reason) => denied(kind, id, &reason),
                }
            }
            Claim::Resume { token } => {
                let gate = self.gate(kind);
                let grant = gate
                    .grants
                    .iter()
                    .find(|grant| code::constant_time_eq(grant.token.as_bytes(), token.as_bytes()))
                    .filter(|grant| grant.method == AccessMethod::Host || gate.still_valid(grant))
                    .cloned();
                match grant {
                    Some(grant) => {
                        if let Some(connection) = self.connections.get_mut(id) {
                            connection.grants[kind as usize] = Some(grant.id.clone());
                            connection.pending[kind as usize] = None;
                        }
                        let mut effects = vec![status(kind, id, AccessState::Granted, None, Some(grant.token))];
                        if kind == AccessKind::Call {
                            effects.push(self.callers_message());
                        }
                        effects.push(Effect::HostChanged);
                        effects
                    }
                    None => denied(kind, id, "Your access has ended"),
                }
            }
            Claim::Host { token } => {
                if kind != AccessKind::Call || !code::constant_time_eq(self.host_token.as_bytes(), token.as_bytes()) {
                    return denied(kind, id, "Not the host");
                }
                self.grant(id, kind, AccessMethod::Host, "Host", None)
            }
        }
    }

    fn claim_with_code(&mut self, id: &str, kind: AccessKind, name: &str, given: &str, now: f64) -> Vec<Effect> {
        if !self.gate(kind).policy.allows(AccessMethod::Code) {
            return denied(kind, id, "Code access is off");
        }
        let Some(address) = self.connections.get(id).map(|c| c.address.clone()) else { return Vec::new() };
        // Each gate counts its own tries: guessing at one never locks people out of the other.
        let gate = self.gate_mut(kind);
        if let Some(seconds) = gate.limiter.wait(&address, now) {
            return denied(kind, id, &format!("Too many attempts. Try again in {seconds} s."));
        }
        if gate.code.as_deref().is_some_and(|expected| code::matches(expected, given)) {
            gate.limiter.succeeded(&address);
            return self.grant(id, kind, AccessMethod::Code, name, None);
        }
        gate.limiter.failed(&address, now);
        let paused = gate.limiter.paused_until(now).is_some();
        let mut effects = denied(kind, id, "Wrong code");
        if let Some(connection) = self.connections.get_mut(id) {
            connection.wrong_codes += 1;
            if connection.wrong_codes >= MAX_TRIES_PER_CONNECTION {
                effects.push(Effect::Close(id.to_owned()));
            }
        }
        if paused {
            effects.push(Effect::HostChanged);
        }
        effects
    }

    fn grant(&mut self, id: &str, kind: AccessKind, method: AccessMethod, name: &str, user_id: Option<String>) -> Vec<Effect> {
        let gate = self.gate_mut(kind);
        let grant = Grant {
            id: Uuid::new_v4().to_string(),
            name: clean_name(name),
            method,
            token: secret(),
            code_generation: gate.code_generation,
            user_id,
        };
        let (grant_id, token) = (grant.id.clone(), grant.token.clone());
        gate.grants.push(grant);
        if let Some(connection) = self.connections.get_mut(id) {
            connection.grants[kind as usize] = Some(grant_id);
            connection.pending[kind as usize] = None;
        }
        let mut effects = vec![status(kind, id, AccessState::Granted, None, Some(token))];
        if kind == AccessKind::Call {
            effects.push(self.callers_message());
        }
        effects.push(Effect::HostChanged);
        effects
    }

    /// Gives up the caller role (or withdraws a request for it).
    fn release(&mut self, id: &str) -> Vec<Effect> {
        let kind = AccessKind::Call;
        let pending = self.connections.get_mut(id).and_then(|c| c.pending[kind as usize].take()).is_some();
        let Some(grant) = self.grant_of(id, kind) else {
            return if pending { vec![status(kind, id, AccessState::Released, None, None), Effect::HostChanged] } else { Vec::new() };
        };
        self.drop_grant(kind, &grant.id);
        vec![status(kind, id, AccessState::Released, None, None), self.callers_message(), Effect::HostChanged]
    }

    /// Removes a grant; returns the connections that were acting under it.
    fn drop_grant(&mut self, kind: AccessKind, grant_id: &str) -> Vec<ConnectionId> {
        self.gate_mut(kind).grants.retain(|grant| grant.id != grant_id);
        let mut affected = Vec::new();
        for id in &self.order {
            if let Some(connection) = self.connections.get_mut(id) {
                if connection.grants[kind as usize].as_deref() == Some(grant_id) {
                    connection.grants[kind as usize] = None;
                    affected.push(id.clone());
                }
            }
        }
        affected
    }

    /// Ends every grant of one gate the predicate picks. Callers are told here; whoever is put
    /// out of the session altogether hears it from `tracking`.
    fn revoke_where(&mut self, kind: AccessKind, reason: &str, predicate: impl Fn(&Grant) -> bool) -> Vec<Effect> {
        let doomed: Vec<String> = self.gate(kind).grants.iter().filter(|g| predicate(g)).map(|g| g.id.clone()).collect();
        let mut effects = Vec::new();
        for grant_id in doomed {
            for connection in self.drop_grant(kind, &grant_id) {
                if kind == AccessKind::Call {
                    effects.push(status(kind, &connection, AccessState::Revoked, Some(reason), None));
                }
            }
        }
        effects
    }

    fn grant_of(&self, id: &str, kind: AccessKind) -> Option<Grant> {
        let grant_id = self.connections.get(id)?.grants[kind as usize].as_deref()?;
        self.gate(kind).grants.iter().find(|grant| grant.id == grant_id).cloned()
    }

    fn caller_name(&self, id: &str) -> Option<String> {
        self.grant_of(id, AccessKind::Call).map(|grant| grant.name)
    }

    /// Who is calling right now (connected callers).
    pub fn callers(&self) -> Vec<CallerInfo> {
        self.call
            .grants
            .iter()
            .filter(|grant| self.is_online(&grant.id))
            .map(|grant| CallerInfo { name: grant.name.clone(), method: grant.method })
            .collect()
    }

    fn is_online(&self, call_grant: &str) -> bool {
        self.connections.values().any(|c| c.grants[AccessKind::Call as usize].as_deref() == Some(call_grant))
    }

    fn callers_message(&self) -> Effect {
        Effect::Broadcast(ServerMessage::Callers { list: self.callers() })
    }

    // -----------------------------------------------------------------------------------------
    // Cues
    // -----------------------------------------------------------------------------------------

    fn refuse_control(&self, id: &str) -> Vec<Effect> {
        vec![Effect::Send(id.to_owned(), ServerMessage::Error { message: "Only a show caller can do that".into() })]
    }

    /// Next or previous, ignored when the show already moved from what the caller saw.
    fn step(&mut self, id: &str, from_cue: Option<i64>, now: f64, forward: bool) -> Vec<Effect> {
        let Some(by) = self.caller_name(id) else { return self.refuse_control(id) };
        if from_cue != self.current_cue {
            // Someone else got there first: just bring this caller up to date.
            return match self.current_cue {
                Some(current) => vec![Effect::Send(id.to_owned(), self.cue_message(current, None))],
                None => Vec::new(),
            };
        }
        let target = match self.current_cue {
            None => 1,
            Some(current) if forward => current + 1,
            Some(current) => (current - 1).max(1),
        };
        vec![self.change_cue(target, forward, by, now), Effect::HostChanged]
    }

    fn cue_message(&self, cue: i64, by: Option<String>) -> ServerMessage {
        ServerMessage::CueChange {
            current_cue: cue,
            next_cue: Some(cue + 1),
            show_started_at: self.show_started_at,
            cue_started_at: self.cue_started_at,
            by,
        }
    }

    fn change_cue(&mut self, cue: i64, start_show: bool, by: String, now: f64) -> Effect {
        self.current_cue = Some(cue);
        self.cue_started_at = Some(now);
        if start_show && self.show_started_at.is_none() {
            self.show_started_at = Some(now);
        }
        Effect::Broadcast(self.cue_message(cue, Some(by)))
    }

    // -----------------------------------------------------------------------------------------
    // Host actions (only ever reachable from inside the host app, never over the socket)
    // -----------------------------------------------------------------------------------------

    /// Everyone connected hears about new rules, including those still waiting to get in.
    fn policy_messages(&self) -> Vec<Effect> {
        let message = ServerMessage::Policy { policy: self.call.policy, view_policy: self.view.policy };
        self.order.iter().map(|id| Effect::Send(id.clone(), message.clone())).collect()
    }

    fn change_policy(&mut self, kind: AccessKind, policy: AccessPolicy) -> Vec<Effect> {
        let gate = self.gate_mut(kind);
        gate.policy = policy.normalized();
        if gate.policy.code && gate.code.is_none() {
            gate.code = Some(code::generate(code::DEFAULT_GENERATED_CHARS));
        }
        let allowed = gate.policy;
        let reason = match kind {
            AccessKind::View => "The host changed who may join",
            AccessKind::Call => "The host changed who may call the show",
        };
        let mut effects = self.revoke_where(kind, reason, |grant| !allowed.allows(grant.method));
        // Opening the door answers the people waiting at it; `tracking` lets them in.
        if !allowed.request && !(kind == AccessKind::View && allowed.open) {
            for id in self.order.clone() {
                if self.connections.get_mut(&id).and_then(|c| c.pending[kind as usize].take()).is_some() {
                    effects.extend(denied(kind, &id, "Requests are off"));
                }
            }
        }
        effects
    }

    pub fn set_policy(&mut self, kind: AccessKind, policy: AccessPolicy) -> Vec<Effect> {
        self.tracking("The host changed who may join", |logic| {
            let mut effects = logic.change_policy(kind, policy);
            effects.extend(logic.policy_messages());
            effects.push(logic.callers_message());
            effects.push(Effect::HostChanged);
            effects
        })
    }

    fn change_code(&mut self, kind: AccessKind, code: String) -> Vec<Effect> {
        let gate = self.gate_mut(kind);
        gate.code = Some(code);
        gate.code_generation += 1;
        gate.limiter.reset();
        self.revoke_where(kind, "The caller code was changed", |grant| grant.method == AccessMethod::Code)
    }

    /// Sets a typed code, or generates one of `length` hex characters. Whoever came through with
    /// the old code loses that access.
    pub fn set_code(&mut self, kind: AccessKind, typed: Option<String>, length: Option<usize>) -> Result<Vec<Effect>, CodeError> {
        let next = match typed {
            Some(code) => {
                code::validate(&code)?;
                code
            }
            None => code::generate(length.unwrap_or(code::DEFAULT_GENERATED_CHARS)),
        };
        Ok(self.tracking("The access code was changed", |logic| {
            let mut effects = logic.change_code(kind, next);
            effects.push(logic.callers_message());
            effects.push(Effect::HostChanged);
            effects
        }))
    }

    /// New settings for a running session, applied in one step. A code is only replaced (ending
    /// the access of those who used it) when a different one is given.
    pub fn apply_settings(&mut self, settings: AccessSettings) -> Result<Vec<Effect>, CodeError> {
        let codes = [settings.view_code, settings.caller_code];
        for code in codes.iter().flatten() {
            code::validate(code)?;
        }
        let policies = [settings.view, settings.caller];
        Ok(self.tracking("The host changed who may join", |logic| {
            let mut effects = Vec::new();
            for (kind, (policy, code)) in KINDS.into_iter().zip(policies.into_iter().zip(codes)) {
                if policy.normalized() != logic.gate(kind).policy {
                    effects.extend(logic.change_policy(kind, policy));
                }
                if let Some(code) = code.filter(|new| logic.gate(kind).code.as_deref() != Some(new.as_str())) {
                    effects.extend(logic.change_code(kind, code));
                }
            }
            effects.extend(logic.policy_messages());
            effects.push(logic.callers_message());
            effects.push(Effect::HostChanged);
            effects
        }))
    }

    pub fn answer_request(&mut self, client_id: &str, kind: AccessKind, approve: bool) -> Vec<Effect> {
        self.tracking("The host declined the request", |logic| {
            let Some(name) = logic.connections.get_mut(client_id).and_then(|c| c.pending[kind as usize].take()) else { return Vec::new() };
            if approve && logic.gate(kind).policy.allows(AccessMethod::Request) {
                logic.grant(client_id, kind, AccessMethod::Request, &name, None)
            } else {
                let mut effects = denied(kind, client_id, "The host declined the request");
                effects.push(Effect::HostChanged);
                effects
            }
        })
    }

    /// The host learned who belongs to the show now: account grants that are no longer covered
    /// end. Changes nothing (and reports nothing) when everyone still may.
    pub fn retain_accounts(&mut self, allowed: impl Fn(&str, AccessKind) -> bool) -> Vec<Effect> {
        let doomed = |logic: &Self, kind| {
            logic.gate(kind).grants.iter().any(|g| g.user_id.as_deref().is_some_and(|user| !allowed(user, kind)))
        };
        if !KINDS.into_iter().any(|kind| doomed(self, kind)) {
            return Vec::new();
        }
        self.tracking("You are no longer a member of this show", |logic| {
            let mut effects = Vec::new();
            for (kind, reason) in KINDS.into_iter().zip(["", "You are no longer a caller of this show"]) {
                effects.extend(logic.revoke_where(kind, reason, |grant| {
                    grant.user_id.as_deref().is_some_and(|user| !allowed(user, kind))
                }));
            }
            effects.push(logic.callers_message());
            effects.push(Effect::HostChanged);
            effects
        })
    }

    /// Takes the caller role from one grant.
    pub fn revoke(&mut self, grant_id: &str) -> Vec<Effect> {
        self.tracking("The host removed you", |logic| {
            let mut effects = logic.revoke_where(AccessKind::Call, "The host removed your caller access", |grant| grant.id == grant_id);
            effects.push(logic.callers_message());
            effects.push(Effect::HostChanged);
            effects
        })
    }

    /// Puts a device out of a protected session: every grant it holds ends, tokens included.
    /// (While joining is open it can simply come back.)
    pub fn remove_device(&mut self, client_id: &str) -> Vec<Effect> {
        self.tracking("The host removed you from the session", |logic| {
            let Some(connection) = logic.connections.get_mut(client_id) else { return Vec::new() };
            connection.pending = Default::default();
            let held = connection.grants.clone();
            let mut effects = Vec::new();
            for (kind, grant) in KINDS.into_iter().zip(held) {
                if let Some(grant) = grant {
                    effects.extend(logic.revoke_where(kind, "The host removed your caller access", |g| g.id == grant));
                }
            }
            effects.push(logic.callers_message());
            effects.push(Effect::HostChanged);
            effects
        })
    }

    pub fn host_view(&self, now: f64) -> HostView {
        let live = |id: &ConnectionId| self.connections.get(id).map(|c| (id.clone(), c));
        let paused = |gate: &Gate| gate.limiter.paused_until(now).map(|until| (until - now).ceil() as u64);
        HostView {
            settings: self.settings(),
            view_paused_seconds: paused(&self.view),
            call_paused_seconds: paused(&self.call),
            current_cue: self.current_cue,
            people: self
                .order
                .iter()
                .filter_map(live)
                .map(|(client_id, c)| {
                    // Someone who has not joined as crew is still known by the name they gave to get in.
                    let name = c.name.clone().or_else(|| self.grant_of(&client_id, AccessKind::View).map(|g| g.name));
                    let admitted = self.is_admitted(&client_id);
                    let caller = self.grant_of(&client_id, AccessKind::Call);
                    HostPerson {
                        client_id,
                        name,
                        role_id: c.role.clone(),
                        admitted,
                        is_caller: caller.is_some(),
                        is_host: caller.is_some_and(|grant| grant.method == AccessMethod::Host),
                    }
                })
                .collect(),
            callers: self
                .call
                .grants
                .iter()
                .map(|g| HostCaller { grant_id: g.id.clone(), name: g.name.clone(), method: g.method, online: self.is_online(&g.id) })
                .collect(),
            requests: self
                .order
                .iter()
                .filter_map(live)
                .flat_map(|(client_id, c)| {
                    KINDS.into_iter().filter_map(move |kind| {
                        c.pending[kind as usize].clone().map(|name| HostRequest { client_id: client_id.clone(), name, kind })
                    })
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests;
