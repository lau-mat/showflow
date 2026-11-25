//! Finding running sessions on the local network. A ShowFlow app broadcasts a **query** over UDP;
//! every computer hosting sessions answers it directly with an **announcement**. Neither carries
//! anything secret, and the host's address is taken from the packet, never from its content.

use serde::{Deserialize, Serialize};

/// The UDP port hosts listen on, just below the session port range.
pub const DISCOVERY_PORT: u16 = 47099;
/// Bumped when the messages below change shape. (The session protocol has its own version.)
pub const DISCOVERY_VERSION: u32 = 1;
/// Long names are cut so that a host with many sessions still fits one datagram.
const MAX_NAME_CHARS: usize = 80;
const MAX_SESSIONS: usize = 50;
/// Larger than any announcement a host builds.
pub const MAX_DATAGRAM_BYTES: usize = 32 * 1024;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AnnouncedSession {
    pub session_id: String,
    pub show_name: String,
    /// The session's web port: with the sender's address, the join address.
    pub port: u16,
    /// The live-session protocol this host speaks (`session::PROTOCOL_VERSION`).
    pub protocol: u32,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "showflow", rename_all = "snake_case")]
enum Message {
    Discover { v: u32 },
    Sessions { v: u32, sessions: Vec<AnnouncedSession> },
}

fn encode(message: &Message) -> Vec<u8> {
    // Strings, integers and lists of them always serialize.
    serde_json::to_vec(message).expect("a discovery message serializes")
}

pub fn query() -> Vec<u8> {
    encode(&Message::Discover { v: DISCOVERY_VERSION })
}

pub fn is_query(datagram: &[u8]) -> bool {
    matches!(serde_json::from_slice(datagram), Ok(Message::Discover { v: DISCOVERY_VERSION }))
}

pub fn announcement(sessions: &[AnnouncedSession]) -> Vec<u8> {
    let sessions = sessions
        .iter()
        .take(MAX_SESSIONS)
        .map(|session| AnnouncedSession { show_name: clean(&session.show_name), ..session.clone() })
        .collect();
    encode(&Message::Sessions { v: DISCOVERY_VERSION, sessions })
}

fn clean(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(MAX_NAME_CHARS).collect()
}

/// The sessions in an announcement. Anyone on the network can send one, so it is treated as
/// untrusted: names are cleaned again, impossible entries dropped.
pub fn read_announcement(datagram: &[u8]) -> Option<Vec<AnnouncedSession>> {
    if datagram.len() > MAX_DATAGRAM_BYTES {
        return None;
    }
    match serde_json::from_slice(datagram) {
        Ok(Message::Sessions { v: DISCOVERY_VERSION, sessions }) => Some(
            sessions
                .into_iter()
                .take(MAX_SESSIONS)
                .filter(|session| session.port != 0 && !session.session_id.is_empty() && session.session_id.len() <= 64)
                .map(|session| AnnouncedSession { show_name: clean(&session.show_name), ..session })
                .collect(),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(name: &str, port: u16) -> AnnouncedSession {
        AnnouncedSession { session_id: "s1".into(), show_name: name.into(), port, protocol: 2 }
    }

    #[test]
    fn queries_and_announcements_round_trip_and_are_not_confused() {
        assert!(is_query(&query()));
        let reply = announcement(&[session("Gala", 47100)]);
        assert_eq!(read_announcement(&reply), Some(vec![session("Gala", 47100)]));
        assert!(!is_query(&reply));
        assert_eq!(read_announcement(&query()), None);
        for junk in [&b""[..], b"hello", b"{}", br#"{"showflow":"discover","v":99}"#] {
            assert!(!is_query(junk));
            assert_eq!(read_announcement(junk), None);
        }
    }

    #[test]
    fn what_comes_off_the_network_is_trimmed_and_bounded() {
        // Not built with `announcement`, which would already clean it: any device can send this.
        let hostile = serde_json::to_vec(&Message::Sessions {
            v: DISCOVERY_VERSION,
            sessions: vec![session(&format!("a\n\u{7}{}", "x".repeat(500)), 47100), session("no port", 0)],
        })
        .unwrap();
        let read = read_announcement(&hostile).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].show_name.chars().count(), MAX_NAME_CHARS);
        assert!(!read[0].show_name.contains(['\n', '\u{7}']));
        assert_eq!(read_announcement(&vec![b' '; MAX_DATAGRAM_BYTES + 1]), None);
    }

    #[test]
    fn a_full_house_still_fits_one_datagram() {
        let many: Vec<_> = (0..200).map(|_| session(&"é".repeat(300), 47100)).collect();
        let reply = announcement(&many);
        assert!(reply.len() <= MAX_DATAGRAM_BYTES, "{}", reply.len());
        assert_eq!(read_announcement(&reply).unwrap().len(), MAX_SESSIONS);
    }
}
