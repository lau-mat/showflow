//! The network side of discovery (`protocol::discovery`): answering queries while this computer
//! hosts, and searching for other hosts. Plain UDP broadcast, no extra dependency.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use protocol::discovery::{announcement, is_query, query, read_announcement, AnnouncedSession, MAX_DATAGRAM_BYTES};
use protocol::session::PROTOCOL_VERSION;
use serde::Serialize;
use tokio::net::UdpSocket;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{timeout_at, Instant};

/// What this computer currently has on offer.
pub type Announced = Arc<Mutex<Vec<AnnouncedSession>>>;

/// How long a search listens for answers.
pub const SEARCH_TIME: Duration = Duration::from_millis(1200);

/// Only devices on the local network get an answer. A query with a forged outside address would
/// otherwise make this computer send its (larger) answer to a stranger.
fn is_local(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
        IpAddr::V6(ip) => ip.is_loopback(),
    }
}

/// Answers queries for as long as it exists.
pub struct Responder {
    task: JoinHandle<()>,
}

impl Responder {
    pub async fn start(port: u16, sessions: Announced) -> Result<Self, String> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port))
            .await
            .map_err(|error| format!("UDP port {port} is not available: {error}"))?;
        let task = tokio::spawn(async move {
            let mut buffer = [0u8; 512];
            loop {
                let Ok((length, from)) = socket.recv_from(&mut buffer).await else {
                    // Some systems report an earlier failed send here; keep listening.
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    continue;
                };
                if !is_local(from.ip()) || !is_query(&buffer[..length]) {
                    continue;
                }
                let reply = {
                    let sessions = sessions.lock().unwrap();
                    (!sessions.is_empty()).then(|| announcement(&sessions))
                };
                if let Some(reply) = reply {
                    let _ = socket.send_to(&reply, from).await;
                }
            }
        });
        Ok(Self { task })
    }
}

impl Drop for Responder {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FoundSession {
    pub host: String,
    pub port: u16,
    pub session_id: String,
    pub show_name: String,
    /// False when the host runs a ShowFlow version this one cannot talk to.
    pub compatible: bool,
}

/// One socket to ask from, and where to send the question.
#[derive(Debug, PartialEq)]
pub struct Route {
    pub from: Ipv4Addr,
    pub to: Vec<Ipv4Addr>,
}

/// A plain broadcast leaves through one network only, and AV computers often sit on several
/// (control, Wi-Fi, audio). So the question is asked once from every address this computer has:
/// to the broadcast address, and to the broadcast address of its /24 (the usual subnet size),
/// which is routed to the right network on every system. This computer itself is asked too.
pub fn routes(addresses: impl IntoIterator<Item = IpAddr>) -> Vec<Route> {
    let mut routes = vec![
        Route { from: Ipv4Addr::LOCALHOST, to: vec![Ipv4Addr::LOCALHOST] },
        Route { from: Ipv4Addr::UNSPECIFIED, to: vec![Ipv4Addr::BROADCAST] },
    ];
    for address in addresses {
        let IpAddr::V4(ip) = address else { continue };
        if ip.is_loopback() || ip.is_unspecified() || routes.iter().any(|route| route.from == ip) {
            continue;
        }
        let [a, b, c, _] = ip.octets();
        routes.push(Route { from: ip, to: vec![Ipv4Addr::BROADCAST, Ipv4Addr::new(a, b, c, 255)] });
    }
    routes
}

fn local_addresses() -> Vec<IpAddr> {
    local_ip_address::list_afinet_netifas()
        .map(|interfaces| interfaces.into_iter().map(|(_, address)| address).collect())
        .unwrap_or_default()
}

/// Asks from one route and collects answers until `until`. A route that cannot be used (an
/// interface that is down, say) just finds nothing.
async fn ask(route: Route, port: u16, until: Instant) -> Vec<(IpAddr, AnnouncedSession)> {
    let mut found = Vec::new();
    let Ok(socket) = UdpSocket::bind((route.from, 0)).await else { return found };
    if socket.set_broadcast(true).is_err() {
        return found;
    }
    let question = query();
    let send = || async {
        for to in &route.to {
            let _ = socket.send_to(&question, SocketAddr::from((*to, port))).await;
        }
    };
    send().await;
    // UDP may drop a packet, Wi-Fi broadcasts especially: ask once more along the way.
    let again = Instant::now() + until.saturating_duration_since(Instant::now()) / 3;
    let mut repeated = false;
    let mut buffer = vec![0u8; MAX_DATAGRAM_BYTES];
    loop {
        let deadline = if repeated { until } else { again };
        match timeout_at(deadline, socket.recv_from(&mut buffer)).await {
            Ok(Ok((length, from))) => {
                for session in read_announcement(&buffer[..length]).unwrap_or_default() {
                    found.push((from.ip(), session));
                }
            }
            Ok(Err(_)) => tokio::time::sleep(Duration::from_millis(50)).await,
            Err(_) if repeated => break,
            Err(_) => {
                send().await;
                repeated = true;
            }
        }
        if Instant::now() >= until {
            break;
        }
    }
    found
}

/// A host reachable over several networks answers several times: one entry per session, with a
/// network address rather than "this computer" when there is a choice.
fn merge(answers: Vec<(IpAddr, AnnouncedSession)>) -> Vec<FoundSession> {
    let mut best: HashMap<String, (IpAddr, AnnouncedSession)> = HashMap::new();
    for (address, session) in answers {
        let better = match best.get(&session.session_id) {
            None => true,
            Some((have, _)) => (address.is_loopback(), address) < (have.is_loopback(), *have),
        };
        if better {
            best.insert(session.session_id.clone(), (address, session));
        }
    }
    let mut found: Vec<FoundSession> = best
        .into_values()
        .map(|(address, session)| FoundSession {
            host: address.to_string(),
            port: session.port,
            session_id: session.session_id,
            show_name: session.show_name,
            compatible: session.protocol == PROTOCOL_VERSION,
        })
        .collect();
    // A stable order, so the list does not jump around between searches.
    found.sort_by(|a, b| (&a.show_name, &a.host, a.port).cmp(&(&b.show_name, &b.host, b.port)));
    found
}

async fn search_routes(routes: Vec<Route>, port: u16, wait: Duration) -> Vec<FoundSession> {
    let until = Instant::now() + wait;
    let mut asking = JoinSet::new();
    for route in routes {
        asking.spawn(ask(route, port, until));
    }
    let mut answers = Vec::new();
    while let Some(result) = asking.join_next().await {
        answers.extend(result.unwrap_or_default());
    }
    merge(answers)
}

/// Every session answering on the local network.
pub async fn search(port: u16, wait: Duration) -> Vec<FoundSession> {
    search_routes(routes(local_addresses()), port, wait).await
}

#[cfg(test)]
mod tests;
