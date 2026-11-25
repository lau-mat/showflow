//! Real sessions on real sockets: gating, several callers, several sessions, ports and origins.

use std::collections::HashMap;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use protocol::session::{AccessKind, AccessPolicy, AccessSettings};
use protocol::sync::{EntityKind, OpKind};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::*;
use crate::session::registry::Sessions;
use crate::{db, store};

/// Not the app's own range, so tests never collide with a session someone is really running.
const RANGE: PortRange = PortRange { from: 47400, to: 47499 };

fn database(shows: &[&str]) -> (Db, Vec<String>) {
    let mut conn = rusqlite::Connection::open_in_memory().unwrap();
    db::migrate(&mut conn).unwrap();
    let ids = shows
        .iter()
        .map(|name| {
            let id = Uuid::new_v4();
            store::apply_local(&mut conn, &id.to_string(), store::new_op(EntityKind::Show, id, OpKind::Set, json!({"name": name, "time": 1}))).unwrap();
            id.to_string()
        })
        .collect();
    (Arc::new(Mutex::new(conn)), ids)
}

fn quiet() -> OnChange {
    Arc::new(|_| {})
}

/// Joining open, calling as given.
fn calling(caller: AccessPolicy, code: Option<&str>) -> AccessSettings {
    AccessSettings { caller, caller_code: code.map(str::to_owned), ..AccessSettings::default() }
}

async fn start(sessions: &Sessions, db: &Db, show: &str, policy: AccessPolicy, code: Option<&str>) -> Arc<RunningSession> {
    start_with(sessions, db, show, calling(policy, code)).await
}

async fn start_with(sessions: &Sessions, db: &Db, show: &str, settings: AccessSettings) -> Arc<RunningSession> {
    sessions.start(show, "Gala".into(), settings, db.clone(), RANGE, quiet()).await.unwrap()
}

fn open() -> AccessPolicy {
    AccessPolicy { open: true, ..Default::default() }
}
fn code_only() -> AccessPolicy {
    AccessPolicy { code: true, ..Default::default() }
}

struct Client {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    welcome: Value,
}

impl Client {
    async fn connect(port: u16) -> Self {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}")).await.expect("connect");
        let mut client = Self { socket, welcome: Value::Null };
        client.welcome = client.expect("welcome").await;
        if client.welcome["view_policy"]["open"] == true {
            // Let in at once: take the show that comes with it, so later reads are fresh.
            client.expect("callers").await;
        }
        client
    }

    async fn send(&mut self, message: Value) {
        self.socket.send(Message::Text(message.to_string())).await.unwrap();
    }

    /// The next message with this `command`, skipping others.
    async fn expect(&mut self, command: &str) -> Value {
        self.find(command, Duration::from_secs(3)).await.unwrap_or_else(|| panic!("no {command} message"))
    }

    async fn find(&mut self, command: &str, wait: Duration) -> Option<Value> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            match tokio::time::timeout_at(deadline, self.socket.next()).await {
                Ok(Some(Ok(Message::Text(text)))) => {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    if value["command"] == command {
                        return Some(value);
                    }
                }
                _ => return None,
            }
        }
    }

    /// True once the server closed the connection.
    async fn closed(&mut self) -> bool {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            match tokio::time::timeout_at(deadline, self.socket.next()).await {
                Ok(None | Some(Err(_)) | Some(Ok(Message::Close(_)))) => return true,
                Ok(Some(Ok(_))) => {}
                Err(_) => return false,
            }
        }
    }

    async fn claim(&mut self, claim: Value) -> Value {
        self.send(json!({"command": "claim_caller", "claim": claim})).await;
        self.expect("caller_status").await
    }

    async fn snapshot(&mut self) -> Value {
        self.send(json!({"command": "get_session_data"})).await;
        self.expect("get_session_data").await["session"].clone()
    }
}

#[tokio::test]
async fn an_account_pass_works_over_the_socket_and_a_replay_does_not() {
    use protocol::caller_pass::{public_key_text, sign_pass, sign_proof, signing_key, PassBody, PASS_VERSION};
    let (db, shows) = database(&["Gala"]);
    let (cloud, device) = (signing_key([1; 32]), signing_key([2; 32]));
    let sam_only = crate::caller_store::ShowAccess { members: vec!["sam".into()], callers: vec!["sam".into()] };
    let lists = HashMap::from([(shows[0].clone(), sam_only)]);
    crate::caller_store::save_trust(&mut db.lock().unwrap(), &public_key_text(&cloud), &lists).unwrap();
    let pass = sign_pass(&cloud, &PassBody {
        v: PASS_VERSION, show_id: shows[0].clone(), user_id: "sam".into(), username: "Sam".into(),
        device_key: public_key_text(&device), issued_at: now() as i64, expires_at: now() as i64 + 600,
    });
    let sessions = Sessions::default();
    let policy = AccessPolicy { account: true, ..Default::default() };
    let session = start(&sessions, &db, &shows[0], policy, None).await;

    let mut sam = Client::connect(session.ws_port).await;
    let proof = sign_proof(&device, &session.session_id, sam.welcome["nonce"].as_str().unwrap());
    let claim = json!({"method": "account", "pass": pass, "proof": proof});

    let mut eavesdropper = Client::connect(session.ws_port).await;
    assert_eq!(eavesdropper.claim(claim.clone()).await["state"], "denied");
    assert_eq!(sam.claim(claim).await["state"], "granted");
    sam.send(json!({"command": "next_cue", "from_cue": null})).await;
    assert_eq!(sam.expect("cue_change").await["by"], "Sam");

    // The next sync takes Sam off the list.
    session.apply(|logic| logic.retain_accounts(|_, _| false));
    assert_eq!(sam.expect("caller_status").await["state"], "revoked");
}

#[tokio::test]
async fn running_sessions_are_announced_and_ended_ones_are_not() {
    use crate::session::discovery;
    let port = std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let (db, shows) = database(&["Gala"]);
    let sessions = Sessions::discoverable(port);
    assert!(!sessions.is_discoverable().await, "nothing listens before there is a session");
    let session = start(&sessions, &db, &shows[0], open(), None).await;
    assert!(sessions.is_discoverable().await);

    // A real search over this computer's interfaces, on a port of our own.
    let search = || discovery::search(port, Duration::from_millis(400));
    let found = search().await;
    let ours = found.iter().find(|f| f.session_id == session.session_id).expect("found");
    assert_eq!((ours.port, ours.show_name.as_str(), ours.compatible), (session.http_port, "Gala", true));
    // The address it was found under really serves the session.
    TcpStream::connect((ours.host.as_str(), ours.port)).await.expect("reachable");

    sessions.stop(&session.session_id).await.unwrap();
    assert!(search().await.iter().all(|f| f.session_id != session.session_id));
}

#[tokio::test]
async fn a_protected_session_sends_an_uninvited_device_nothing_at_all() {
    let (db, shows) = database(&["Secret Gala"]);
    let sessions = Sessions::default();
    let door = AccessSettings {
        view: code_only(),
        view_code: Some("JoinMe!23".into()),
        caller: open(),
        caller_code: None,
    };
    let session = start_with(&sessions, &db, &shows[0], door).await;
    let mut eve = Client::connect(session.ws_port).await;
    let mut ann = Client::connect(session.ws_port).await;
    assert_eq!(ann.welcome["view_policy"]["code"], true);
    assert!(!ann.welcome.to_string().contains("Gala") && !ann.welcome.to_string().contains("JoinMe"));

    for message in [
        json!({"command": "get_show_data"}),
        json!({"command": "get_session_data"}),
        json!({"command": "join_session", "user_name": "Eve", "role_id": null}),
        json!({"command": "claim_caller", "claim": {"method": "open", "name": "Eve"}}),
    ] {
        eve.send(message).await;
    }
    assert_eq!(eve.expect("caller_status").await["state"], "denied");

    ann.send(json!({"command": "request_view", "claim": {"method": "code", "name": "Ann", "code": "JoinMe!23"}})).await;
    let granted = ann.expect("view_status").await;
    assert_eq!(granted["state"], "granted");
    assert_eq!(ann.expect("get_show_data").await["data"]["show"]["name"], "Secret Gala");
    ann.expect("get_session_data").await;
    assert_eq!(ann.claim(json!({"method": "open", "name": "Ann"})).await["state"], "granted");
    ann.send(json!({"command": "next_cue", "from_cue": null})).await;
    ann.expect("cue_change").await;

    // Everything broadcast so far (callers, the cue change) has passed Eve by.
    eve.send(json!({"command": "no_such_thing"})).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let next = tokio::time::timeout_at(deadline, eve.socket.next()).await.expect("the error arrives").unwrap().unwrap();
        let Message::Text(text) = next else { continue };
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["command"], "error", "only the answer to Eve's own message: {value}");
        break;
    }
    session.stop();
}

async fn http_get(port: u16, path: &str) -> (String, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let text = String::from_utf8_lossy(&response).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    (head.to_ascii_lowercase(), body.to_string())
}

#[tokio::test]
async fn only_callers_move_the_show_and_two_presses_advance_it_once() {
    let (db, shows) = database(&["Gala"]);
    let sessions = Sessions::default();
    let session = start(&sessions, &db, &shows[0], open(), None).await;

    let mut ann = Client::connect(session.ws_port).await;
    let mut ben = Client::connect(session.ws_port).await;
    assert_eq!(ann.welcome["protocol"], 2);
    assert_eq!(ann.welcome["session_id"], session.session_id.as_str());
    assert_ne!(ann.welcome["nonce"], ben.welcome["nonce"]);

    // A device that only joined cannot control anything.
    ben.send(json!({"command": "next_cue", "from_cue": null})).await;
    assert!(ben.expect("error").await["message"].as_str().unwrap().contains("show caller"));
    assert_eq!(ben.snapshot().await["current_cue"], Value::Null);

    assert_eq!(ann.claim(json!({"method": "open", "name": "Ann"})).await["state"], "granted");
    assert_eq!(ben.claim(json!({"method": "open", "name": "Ben"})).await["state"], "granted");
    // Both press Next while seeing "not started".
    ann.send(json!({"command": "next_cue", "from_cue": null})).await;
    let first = ann.expect("cue_change").await;
    ben.send(json!({"command": "next_cue", "from_cue": null})).await;
    assert_eq!((first["current_cue"].as_i64(), first["by"].as_str()), (Some(1), Some("Ann")));
    assert_eq!(ben.expect("cue_change").await["current_cue"], 1, "Ben is told where the show is");
    assert_eq!(ann.snapshot().await["current_cue"], 1, "the show advanced once");

    ben.send(json!({"command": "next_cue", "from_cue": 1})).await;
    let second = ann.expect("cue_change").await;
    assert_eq!((second["current_cue"].as_i64(), second["by"].as_str()), (Some(2), Some("Ben")));

    // Show content comes from the local database.
    ann.send(json!({"command": "get_show_data"})).await;
    assert_eq!(ann.expect("get_show_data").await["data"]["show"]["name"], "Gala");
    session.stop();
}

#[tokio::test]
async fn a_code_is_required_and_changing_it_removes_callers_live() {
    let (db, shows) = database(&["Gala"]);
    let sessions = Sessions::default();
    let session = start(&sessions, &db, &shows[0], code_only(), Some("3FA9C1")).await;
    let mut sam = Client::connect(session.ws_port).await;
    assert_eq!(sam.claim(json!({"method": "open", "name": "Sam"})).await["state"], "denied");
    assert_eq!(sam.claim(json!({"method": "code", "name": "Sam", "code": "000000"})).await["reason"], "Wrong code");
    let granted = sam.claim(json!({"method": "code", "name": "Sam", "code": "3fa9c1"})).await;
    assert_eq!(granted["state"], "granted");
    let token = granted["resume_token"].as_str().unwrap().to_owned();

    // A reconnect (reload, Wi-Fi drop) resumes without the code.
    drop(sam);
    let mut again = Client::connect(session.ws_port).await;
    assert_eq!(again.claim(json!({"method": "resume", "token": token})).await["state"], "granted");

    session.act(|logic| match logic.set_code(AccessKind::Call, Some("N3w-C0de!".into()), None) {
        Ok(effects) => (effects, ()),
        Err(error) => panic!("{error}"),
    });
    let revoked = again.expect("caller_status").await;
    assert_eq!(revoked["state"], "revoked");
    again.send(json!({"command": "next_cue", "from_cue": null})).await;
    assert!(again.find("error", Duration::from_secs(3)).await.is_some());
    assert_eq!(again.claim(json!({"method": "resume", "token": token})).await["state"], "denied");
    session.stop();
}

#[tokio::test]
async fn requests_reach_the_host_and_the_change_hook_fires() {
    let (db, shows) = database(&["Gala"]);
    let sessions = Sessions::default();
    let changes = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = changes.clone();
    let session = sessions
        .start(&shows[0], "Gala".into(), calling(AccessPolicy { request: true, ..Default::default() }, None), db.clone(), RANGE,
            Arc::new(move |id: &str| log.lock().unwrap().push(id.to_owned())))
        .await
        .unwrap();
    let mut kim = Client::connect(session.ws_port).await;
    assert_eq!(kim.claim(json!({"method": "request", "name": "Kim"})).await["state"], "pending");
    let request = session.inspect(|logic| logic.host_view(now()).requests.pop()).expect("the host sees the request");
    assert_eq!(request.name, "Kim");
    assert!(changes.lock().unwrap().iter().all(|id| *id == session.session_id) && !changes.lock().unwrap().is_empty());
    session.apply(|logic| logic.answer_request(&request.client_id, request.kind, true));
    assert_eq!(kim.expect("caller_status").await["state"], "granted");
    session.stop();
}

#[tokio::test]
async fn one_machine_runs_several_sessions_that_share_nothing() {
    let (db, shows) = database(&["Gala", "Rehearsal"]);
    let sessions = Sessions::default();
    let gala = start(&sessions, &db, &shows[0], code_only(), Some("GalaCode!")).await;
    let rehearsal = start(&sessions, &db, &shows[1], code_only(), Some("Rehearse1")).await;

    let ports = [gala.http_port, gala.ws_port, rehearsal.http_port, rehearsal.ws_port];
    assert!(ports.iter().all(|p| (RANGE.from..=RANGE.to).contains(p)), "{ports:?}");
    assert_eq!(ports.iter().collect::<std::collections::HashSet<_>>().len(), 4, "{ports:?}");
    assert!(sessions.start(&shows[0], "Gala".into(), calling(open(), None), db.clone(), RANGE, quiet()).await
        .err().unwrap().contains("already has a running session"));
    assert_eq!(sessions.list().await.len(), 2);

    let mut a = Client::connect(gala.ws_port).await;
    let mut b = Client::connect(rehearsal.ws_port).await;
    // The other session's code and tokens are worthless here.
    assert_eq!(b.claim(json!({"method": "code", "name": "x", "code": "GalaCode!"})).await["state"], "denied");
    let token = a.claim(json!({"method": "code", "name": "A", "code": "GalaCode!"})).await["resume_token"].as_str().unwrap().to_owned();
    assert_eq!(b.claim(json!({"method": "resume", "token": token})).await["state"], "denied");
    assert_eq!(b.claim(json!({"method": "host", "token": gala.inspect(|l| l.host_token().to_owned())})).await["state"], "denied");

    a.send(json!({"command": "next_cue", "from_cue": null})).await;
    a.expect("cue_change").await;
    assert!(b.find("cue_change", Duration::from_millis(400)).await.is_none(), "cues stay in their own session");
    assert_eq!(b.snapshot().await["current_cue"], Value::Null);

    // Ending one leaves the other running and frees exactly its ports.
    sessions.stop(&gala.session_id).await.unwrap();
    assert!(a.closed().await);
    tokio::time::sleep(Duration::from_millis(200)).await;
    for port in [gala.http_port, gala.ws_port] {
        assert!(tokio::net::TcpListener::bind(("0.0.0.0", port)).await.is_ok(), "port {port} should be free again");
    }
    assert!(tokio::net::TcpListener::bind(("0.0.0.0", rehearsal.ws_port)).await.is_err());
    assert_eq!(b.snapshot().await["session_id"], rehearsal.session_id.as_str());
    assert_eq!(sessions.list().await.len(), 1);
    assert!(sessions.get(&gala.session_id).await.is_err() && sessions.stop(&gala.session_id).await.is_err());
    // The stopped show can be started again.
    let restarted = start(&sessions, &db, &shows[0], open(), None).await;
    restarted.stop();
    rehearsal.stop();
}

#[tokio::test]
async fn the_web_server_says_where_the_socket_is_and_serves_the_pages() {
    let (db, shows) = database(&["Gala"]);
    let sessions = Sessions::default();
    let session = start(&sessions, &db, &shows[0], open(), None).await;

    let (head, body) = http_get(session.http_port, "/session-info").await;
    assert!(head.starts_with("http/1.1 200"), "{head}");
    assert!(head.contains("access-control-allow-origin: *"), "the app must be able to ask: {head}");
    let info: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(info, json!({"protocol": 2, "session_id": session.session_id, "show_name": "Gala", "ws_port": session.ws_port}));
    for secret in [session.inspect(|l| l.host_token().to_owned())] {
        assert!(!body.contains(&secret));
    }

    let (head, page) = http_get(session.http_port, "/").await;
    assert!(head.starts_with("http/1.1 200") && head.contains("text/html"), "{head}");
    assert!(page.contains("<html"));
    assert!(http_get(session.http_port, "/session.html").await.0.starts_with("http/1.1 200"));
    assert!(http_get(session.http_port, "/nope.js").await.0.starts_with("http/1.1 404"));
    session.stop();
}

async fn connect_with_origin(port: u16, origin: &str) -> Result<(), String> {
    let mut request = format!("ws://127.0.0.1:{port}").into_client_request().unwrap();
    request.headers_mut().insert("Origin", origin.parse().unwrap());
    tokio_tungstenite::connect_async(request).await.map(|_| ()).map_err(|e| e.to_string())
}

#[tokio::test]
async fn other_web_pages_cannot_open_the_socket() {
    let (db, shows) = database(&["Gala"]);
    let sessions = Sessions::default();
    let session = start(&sessions, &db, &shows[0], open(), None).await;
    assert!(connect_with_origin(session.ws_port, "http://evil.example").await.unwrap_err().contains("403"));
    assert!(connect_with_origin(session.ws_port, &format!("http://127.0.0.1:{}", session.http_port)).await.is_ok());
    assert!(connect_with_origin(session.ws_port, "tauri://localhost").await.is_ok());
    session.stop();
}

#[test]
fn origins_must_come_from_this_host_or_the_app() {
    let host = Some("192.168.1.5:47123");
    assert!(origin_allowed(None, host), "not a browser");
    assert!(origin_allowed(Some("http://192.168.1.5:47180"), host));
    assert!(origin_allowed(Some("http://tauri.localhost"), host));
    assert_eq!(origin_allowed(Some("http://127.0.0.1:1420"), host), cfg!(debug_assertions), "the dev app, in debug builds");
    assert!(origin_allowed(Some("http://[fe80::1]:3000"), Some("[fe80::1]:47100")));
    assert!(origin_allowed(Some("http://MyHost:1"), Some("myhost:2")));
    for bad in ["http://evil.example", "http://192.168.1.50:47180", "http://192.168.1.5.evil.example", "null", ""] {
        assert!(!origin_allowed(Some(bad), host), "{bad}");
    }
    assert!(!origin_allowed(Some("http://192.168.1.5"), None));
}

#[tokio::test]
async fn oversized_and_malformed_messages_do_not_hurt_the_session() {
    let (db, shows) = database(&["Gala"]);
    let sessions = Sessions::default();
    let session = start(&sessions, &db, &shows[0], open(), None).await;
    let mut client = Client::connect(session.ws_port).await;
    client.send(json!({"command": "no_such_thing"})).await;
    assert!(client.expect("error").await["message"].as_str().unwrap().contains("Invalid client message"));
    assert_eq!(client.snapshot().await["is_active"], true, "the connection survives a bad message");
    let huge = "x".repeat(MAX_MESSAGE_BYTES + 1);
    let _ = client.socket.send(Message::Text(huge)).await;
    assert!(client.closed().await, "an oversized message ends that connection");
    assert_eq!(Client::connect(session.ws_port).await.welcome["protocol"], 2, "others carry on");
    session.stop();
}

/// Not a test: serves two real sessions with a demo script, for trying the pages in a browser.
/// `npm run build:session && cargo test -p showfow serve_demo_sessions -- --ignored --nocapture`
/// Joining is open for A and B: calling takes the code `P@ssw0rd!` in A and nothing in B. Session C
/// needs the code `JoinMe!23` to join, and approves caller requests by itself.
/// `SHOWFLOW_DEMO_SECONDS` sets how long they stay up, `SHOWFLOW_DEMO_FILE` receives their ports.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "serves demo sessions for manual testing"]
async fn serve_demo_sessions() {
    let (db, shows) = database(&["Annual Summit", "Rehearsal", "Board Meeting"]);
    {
        let mut conn = db.lock().unwrap();
        for show in &shows {
            let role = Uuid::new_v4();
            store::apply_local(&mut conn, show, store::new_op(EntityKind::Role, role, OpKind::Set, json!({"name": "Lighting"}))).unwrap();
            let mut after = None;
            for (i, name) in ["Doors open", "Host welcome", "Opening video", "Keynote", "Closing remarks"].iter().enumerate() {
                let line = Uuid::new_v4();
                let mut op = store::new_op(EntityKind::Line, line, OpKind::Set, json!({"name": name, "note": "Stay sharp\nCheck <b>mics</b>", "duration": 60 * (i + 1)}));
                op.place = Some(protocol::sync::Place { after });
                store::apply_local(&mut conn, show, op).unwrap();
                store::apply_local(&mut conn, show, store::new_op(EntityKind::Comment, Uuid::new_v4(), OpKind::Set, json!({"line_id": line, "role_id": role, "comment": "Fade <img src=x onerror=window.__pwned=1> to 20%"}))).unwrap();
                after = Some(line);
            }
        }
    }
    // `SHOWFLOW_DEMO_DISCOVERY` makes them show up in a real app's list (UDP port 47099).
    let sessions = match std::env::var("SHOWFLOW_DEMO_DISCOVERY") {
        Ok(_) => Sessions::discoverable(protocol::discovery::DISCOVERY_PORT),
        Err(_) => Sessions::default(),
    };
    let a = start(&sessions, &db, &shows[0], code_only(), Some("P@ssw0rd!")).await;
    let b = start(&sessions, &db, &shows[1], open(), None).await;
    let locked = AccessSettings {
        view: AccessPolicy { code: true, ..Default::default() },
        view_code: Some("JoinMe!23".into()),
        ..AccessSettings::default()
    };
    let c = start_with(&sessions, &db, &shows[2], locked).await;
    let ports = json!({"a": a.http_port, "b": b.http_port, "c": c.http_port});
    println!("Demo sessions: {ports}");
    if let Ok(file) = std::env::var("SHOWFLOW_DEMO_FILE") {
        std::fs::write(file, ports.to_string()).unwrap();
    }
    let seconds: u64 = std::env::var("SHOWFLOW_DEMO_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(600);
    let until = tokio::time::Instant::now() + Duration::from_secs(seconds);
    while tokio::time::Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(400)).await;
        // Stands in for the host pressing "Approve" in the panel.
        for request in c.inspect(|logic| logic.host_view(now()).requests) {
            c.apply(|logic| logic.answer_request(&request.client_id, request.kind, true));
        }
    }
    a.stop();
    b.stop();
    c.stop();
}
