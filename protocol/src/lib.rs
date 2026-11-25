//! Shared protocol types for the desktop app and cloud backend.
//!
//! Everything that must mean the same thing on every client lives here: the show records the
//! UI renders, the ops every edit is expressed as, member roles and what they may do, and the
//! fractional position keys that keep script lines ordered while several people insert.

pub mod position;
pub mod caller_pass;
pub mod discovery;
pub mod session;
pub mod show;
pub mod sync;
