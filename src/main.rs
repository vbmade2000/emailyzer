use crate::{
    database::{create_email_entry, create_or_open_db},
    email::{
        get_client, get_credentials, get_datetime, get_receiver, get_sender, get_session,
        get_subject, is_seen,
    },
    types::Email,
};

mod database;
mod email;
mod types;

fn main() -> anyhow::Result<()> {
    let conn = create_or_open_db("emailyzer.db")?;

    // Gmail IMAP server.
    let domain = "imap.gmail.com";
    let port = 993;

    let (username, password) = get_credentials()?;

    let client = get_client(domain, port)?;
    println!("Connected successfully.");

    println!("Authenticating...");

    let mut session = get_session(&username, &password, client)?;

    let mailbox = session.examine("INBOX")?;

    println!("INBOX opened in read-only mode.");
    println!("Messages: {}", mailbox.exists);
    println!("Recent messages: {}", mailbox.recent);
    println!("Next UID: {}", mailbox.uid_next.unwrap_or(0));

    let messages = session
        .fetch("1:*", "(UID FLAGS ENVELOPE)")
        .expect("Failed to fetch message metadata");

    println!("\nMessages:");

    for message in messages.iter().rev().take(100) {
        // Unique ID for every email. There is also sequence number but don't depend on it because it changes.
        let uid = message.uid.unwrap_or(0);

        // Extract fields
        let envelope = message.envelope().expect("Server did not return ENVELOPE");
        let _is_seen = is_seen(message);
        let subject = get_subject(envelope);
        let sender = get_sender(envelope);
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

        create_email_entry(&conn, email)?;
    }

    session.logout()?;

    println!("Disconnected successfully.");

    Ok(())
}
