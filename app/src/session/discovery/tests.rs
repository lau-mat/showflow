use super::*;

fn session(id: &str, name: &str, protocol: u32) -> AnnouncedSession {
    AnnouncedSession { session_id: id.into(), show_name: name.into(), port: 47100, protocol }
}

fn loopback() -> Vec<Route> {
    vec![Route { from: Ipv4Addr::LOCALHOST, to: vec![Ipv4Addr::LOCALHOST] }]
}

/// A port nothing else uses (never the real discovery port: a session may really be running).
async fn free_port() -> u16 {
    UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap().local_addr().unwrap().port()
}

#[test]
fn every_network_is_asked_from_its_own_address() {
    let addresses = ["192.168.1.20", "10.4.0.7", "127.0.0.1", "10.4.0.7", "fe80::1"].map(|a| a.parse::<IpAddr>().unwrap());
    let routes = routes(addresses);
    let from: Vec<String> = routes.iter().map(|r| r.from.to_string()).collect();
    assert_eq!(from, ["127.0.0.1", "0.0.0.0", "192.168.1.20", "10.4.0.7"]);
    assert_eq!(routes[2].to, [Ipv4Addr::BROADCAST, Ipv4Addr::new(192, 168, 1, 255)]);
    assert_eq!(routes[3].to, [Ipv4Addr::BROADCAST, Ipv4Addr::new(10, 4, 0, 255)]);
}

#[test]
fn only_the_local_network_is_answered() {
    for local in ["192.168.1.5", "10.0.0.1", "172.16.9.9", "169.254.3.3", "127.0.0.1"] {
        assert!(is_local(local.parse().unwrap()), "{local}");
    }
    for outside in ["8.8.8.8", "172.32.0.1", "193.168.1.1", "2001:db8::1"] {
        assert!(!is_local(outside.parse().unwrap()), "{outside}");
    }
}

#[test]
fn a_host_that_answers_twice_is_listed_once_by_its_network_address() {
    let address = |a: &str| a.parse::<IpAddr>().unwrap();
    let found = merge(vec![
        (address("127.0.0.1"), session("a", "Zulu", PROTOCOL_VERSION)),
        (address("192.168.1.20"), session("a", "Zulu", PROTOCOL_VERSION)),
        (address("127.0.0.1"), session("a", "Zulu", PROTOCOL_VERSION)),
        (address("192.168.1.31"), session("b", "Alpha", PROTOCOL_VERSION + 1)),
    ]);
    let seen: Vec<_> = found.iter().map(|f| (f.show_name.as_str(), f.host.as_str(), f.compatible)).collect();
    assert_eq!(seen, [("Alpha", "192.168.1.31", false), ("Zulu", "192.168.1.20", true)]);
}

#[tokio::test]
async fn a_search_finds_what_the_host_offers_right_now() {
    let port = free_port().await;
    let offered = Announced::default();
    let responder = Responder::start(port, offered.clone()).await.unwrap();
    let wait = Duration::from_millis(300);

    assert!(search_routes(loopback(), port, wait).await.is_empty(), "nothing running: no answer");
    offered.lock().unwrap().push(session("s1", "Gala", PROTOCOL_VERSION));
    let found = search_routes(loopback(), port, wait).await;
    assert_eq!(found, [FoundSession { host: "127.0.0.1".into(), port: 47100, session_id: "s1".into(), show_name: "Gala".into(), compatible: true }]);

    assert!(Responder::start(port, offered.clone()).await.is_err(), "the port is taken");
    drop(responder);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(search_routes(loopback(), port, wait).await.is_empty());
}
