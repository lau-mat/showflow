//! Live show sessions: `server` is the network side of one session, `registry` keeps the ones this
//! computer runs, `discovery` finds them across the network, `commands` is how the app manages them. The show logic itself is the `showlogic`
//! crate.

pub mod commands;
pub mod discovery;
pub mod ports;
pub mod registry;
pub mod server;

pub use registry::Sessions;
