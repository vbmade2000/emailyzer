/// Represents an email message
#[derive(Debug)]
pub struct Email {
    pub uid: u32,
    pub subject: String,
    pub sender: String,
    pub read_status: bool,
    pub receiver: String,
    pub attachment: bool,
    pub timestamp: String,
    pub body: String,
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

/// Database operations
pub enum DatabaseOperations {
    CreateEmailEntry(Email),
    Exit,
}
