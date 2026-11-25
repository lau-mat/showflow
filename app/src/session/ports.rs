//! Each session takes two ports picked at random from a fixed range: no clashes with whatever else
//! uses 8080, room for several sessions on one machine, and a single firewall rule to open.

use tokio::net::TcpListener;
use uuid::Uuid;

pub const DEFAULT_RANGE: PortRange = PortRange { from: 47100, to: 47199 };
const ENV: &str = "SHOWFLOW_SESSION_PORTS";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortRange {
    pub from: u16,
    pub to: u16,
}

impl PortRange {
    /// `"47100-47199"`; needs at least two ports.
    pub fn parse(value: &str) -> Option<Self> {
        let (from, to) = value.split_once('-')?;
        let (from, to) = (from.trim().parse().ok()?, to.trim().parse().ok()?);
        (from > 0 && to > from).then_some(Self { from, to })
    }

    pub fn from_env() -> Self {
        match std::env::var(ENV) {
            Ok(value) => Self::parse(&value).unwrap_or_else(|| {
                eprintln!("Ignoring {ENV}={value:?}: expected <from>-<to>");
                DEFAULT_RANGE
            }),
            Err(_) => DEFAULT_RANGE,
        }
    }

    /// The ports of the range in random order.
    fn shuffled(self) -> Vec<u16> {
        let mut ports: Vec<u16> = (self.from..=self.to).collect();
        for i in (1..ports.len()).rev() {
            let random = u128::from_le_bytes(*Uuid::new_v4().as_bytes());
            ports.swap(i, (random % (i as u128 + 1)) as usize);
        }
        ports
    }
}

/// Binds two free ports from the range. A port is only "taken" if binding it actually fails, and
/// both are held before this returns, so nothing can grab them in between.
pub async fn bind_two(range: PortRange) -> Result<(TcpListener, TcpListener), String> {
    let mut bound = Vec::with_capacity(2);
    for port in range.shuffled() {
        if let Ok(listener) = TcpListener::bind(("0.0.0.0", port)).await {
            bound.push(listener);
            if bound.len() == 2 {
                break;
            }
        }
    }
    match (bound.pop(), bound.pop()) {
        (Some(second), Some(first)) => Ok((first, second)),
        _ => Err(format!(
            "No two free ports between {} and {}. Stop another session or set {ENV}.",
            range.from, range.to
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A range nothing else uses, distinct per test so parallel tests do not collide.
    fn range(from: u16, count: u16) -> PortRange {
        PortRange { from, to: from + count - 1 }
    }

    #[test]
    fn parses_a_range_and_rejects_nonsense() {
        assert_eq!(PortRange::parse("47100-47199"), Some(DEFAULT_RANGE));
        assert_eq!(PortRange::parse(" 5000 - 5001 "), Some(PortRange { from: 5000, to: 5001 }));
        for bad in ["", "5000", "5000-5000", "5001-5000", "0-10", "a-b", "70000-70010"] {
            assert_eq!(PortRange::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn shuffling_keeps_every_port_and_changes_the_order() {
        let all: Vec<u16> = (47100..=47199).collect();
        let mut seen_orders = std::collections::HashSet::new();
        for _ in 0..5 {
            let mut shuffled = DEFAULT_RANGE.shuffled();
            seen_orders.insert(shuffled.clone());
            shuffled.sort_unstable();
            assert_eq!(shuffled, all);
        }
        assert!(seen_orders.len() > 1);
    }

    #[tokio::test]
    async fn binds_two_ports_inside_the_range_skipping_taken_ones() {
        let range = range(47300, 6);
        let taken = TcpListener::bind(("0.0.0.0", 47302)).await.unwrap();
        for _ in 0..10 {
            let (a, b) = bind_two(range).await.unwrap();
            let ports = [a.local_addr().unwrap().port(), b.local_addr().unwrap().port()];
            assert_ne!(ports[0], ports[1]);
            assert!(ports.iter().all(|p| (range.from..=range.to).contains(p) && *p != 47302), "{ports:?}");
        }
        drop(taken);
    }

    #[tokio::test]
    async fn a_full_range_gives_a_clear_error() {
        let range = range(47310, 3);
        let _a = TcpListener::bind(("0.0.0.0", 47310)).await.unwrap();
        let _b = TcpListener::bind(("0.0.0.0", 47311)).await.unwrap();
        let error = bind_two(range).await.unwrap_err();
        assert!(error.contains("47310") && error.contains("47312") && error.contains(ENV), "{error}");
    }
}
