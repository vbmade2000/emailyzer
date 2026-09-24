use std::fmt::Display;

/// Represents an email provider
#[derive(Debug, Default)]
pub struct Provider {
    pub name: String,
    pub url: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    /// Mailbox/label name for the inbox, e.g. "INBOX" for Gmail.
    pub inbox_label: String,
    /// Mailbox/label name for sent emails, e.g. "[Gmail]/Sent Mail" for Gmail.
    pub sent_label: String,
}

impl Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}
