//! Outgoing email seam. Nothing sends mail yet; verification and password reset will go
//! through `Mailer`, so adding a real provider later means one new implementation.
#![allow(dead_code)]

use axum::async_trait;

#[derive(Debug)]
pub struct MailError(pub String);

#[async_trait]
pub trait Mailer: Send + Sync {
    async fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), MailError>;
}

/// Development mailer: prints the message (including any token link) to stdout.
pub struct LogMailer;

#[async_trait]
impl Mailer for LogMailer {
    async fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), MailError> {
        println!("[mail] to={to} subject={subject:?}\n{body}");
        Ok(())
    }
}
