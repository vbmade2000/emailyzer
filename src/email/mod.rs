#[allow(clippy::module_inception)]
pub mod email;
pub mod types;

pub use email::{get_provider_mailboxes, get_receiver_stats, get_sender_stats, sync_emails};
pub use types::{Email, ReceiverStats, SenderStats};
