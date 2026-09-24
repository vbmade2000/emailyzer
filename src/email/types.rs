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

// Application wide settings
pub const DEFAULT_PROVIDER_KEY: &str = "default_provider";
