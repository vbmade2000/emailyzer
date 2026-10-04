pub mod imap_client;
pub mod parsing;
pub mod stats;
pub mod sync;
pub mod tests;
pub mod types;

pub use imap_client::get_provider_mailboxes;
pub use stats::{get_receiver_stats, get_sender_stats};
pub use sync::sync_emails;
pub use types::{Email, ReceiverStats, SenderStats};
