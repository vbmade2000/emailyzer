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
