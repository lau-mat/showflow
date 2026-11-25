//! The show logic of a live session: cues and their timing, who may call the show, and the rules
//! for becoming a caller. It does no I/O. Whoever hosts it (today the desktop app, later a cloud
//! container) feeds it connections and messages and carries out the returned `Effect`s.

pub mod code;
pub mod limiter;
mod session;

pub use session::*;
