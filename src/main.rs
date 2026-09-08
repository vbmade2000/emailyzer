use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::{
    database::{create_email_entry, create_or_open_db, get_last_fetched_uid},
    email::{
        get_client, get_credentials, get_datetime, get_receiver, get_sender, get_session,
        get_subject, is_seen,
    },
    types::Email,
};

mod database;
mod email;
mod types;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let subscriber = tracing_subscriber::fmt()
        .with_file(true)
        .with_line_number(true)
        .with_thread_ids(false)
        .with_target(true)
        .with_env_filter(EnvFilter::from_default_env())
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let conn = create_or_open_db("emailyzer.db").await?;

    // Gmail IMAP server.
    let domain = "imap.gmail.com";
    let port = 993;

    let (username, password) = get_credentials().await?;

    let client = get_client(domain, port).await?;
    info!("Connected successfully.");

    let _last_uid = get_last_fetched_uid(&conn).await?;

    info!("Authenticating...");
    let mut session = get_session(&username, &password, client).await?;

    let _mailbox = session.examine("INBOX")?;

    // let uid_next = mailbox.uid_next.unwrap_or(0);

    // println!("Messages in INBOX: {}", mailbox.exists);
    // println!("Next UID: {}", uid_next);
    // println!("Last UID in DB: {}", last_uid);

    // println!("INBOX opened in read-only mode.");
    // println!("Messages: {}", mailbox.exists);
    // println!("Recent messages: {}", mailbox.recent);
    // println!("Next UID: {}", mailbox.uid_next.unwrap_or(0));

    // let messages = session.uid_fetch(format!("{}:*", last_uid + 1), "(UID FLAGS ENVELOPE)")?;
    let messages = session.uid_fetch("1:*", "(UID FLAGS ENVELOPE)")?;

    info!("Fetching {} messages", messages.len());

    for message in messages.iter().take(10) {
        // Unique ID for every email. There is also sequence number but don't depend on it because it changes.
        let uid = message.uid.unwrap_or(0);

        // Extract fields
        let envelope = message.envelope().expect("Server did not return ENVELOPE");
        let _is_seen = is_seen(message);
        let subject = get_subject(envelope).await;
        let sender = get_sender(envelope).await;
        let receiver = get_receiver(envelope);
        let datetime = get_datetime(envelope);

        let email = Email {
            uid,
            subject,
            sender,
            receiver,
            attachment: false,
            timestamp: datetime,
            body: "".to_string(),
        };

        create_email_entry(&conn, email).await?;
    }

    session.logout()?;

    info!("Session disconnected successfully.");

    Ok(())
}
