use std::fmt::Display;

/// Represents an email message
#[derive(Debug)]
pub struct Email {
    pub uid: u32,
    pub subject: String,
    pub sender: String,
    pub read_status: bool,
    pub receiver: String,
    /// Attachment status: `1` = has attachment, `0` = no attachment, `-1` = unknown (the
    /// `BODYSTRUCTURE` fetch for this UID's batch failed to parse and no fallback was attempted).
    pub attachment: i16,
    pub timestamp: String,
    pub body: String,
    pub label: String,
    pub provider: String,
}

/// Represents sender email stats for a single sender
#[derive(Debug, Default)]
pub struct SenderStats {
    pub sender: String,
    pub total_emails: u32,
    pub read_emails: u32,
    pub unread_emails: u32,
    pub attachment_count: u32,
    pub no_attachment_count: u32,
}

/// Represents sender email stats for a single sender
#[derive(Debug, Default)]
pub struct ReceiverStats {
    pub receiver: String,
    pub total_emails: u32,
    pub read_emails: u32,
    pub unread_emails: u32,
    pub attachment_count: u32,
    pub no_attachment_count: u32,
}

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

/// Database operations
pub enum DatabaseOperations {
    CreateEmailEntry(Email),
}

// Contants
pub const DATABASE_URL: &str = "emailyzer.db";

// Application wide settings
pub const DEFAULT_PROVIDER_KEY: &str = "default_provider";
