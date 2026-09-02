use crate::database::create_or_open_db;

mod database;

use std::collections::HashMap;

use native_tls::TlsConnector;
use tokio_imap::types::Address;

fn main() -> anyhow::Result<()> {

    let _conn = create_or_open_db("emailyzer.db")?;

    // Gmail IMAP server.
    let domain = "imap.gmail.com";
    let port = 993;

    // Read credentials from environment variables.
    //
    // Do NOT hardcode your Gmail password or App Password in source code.
    let username =
        std::env::var("GMAIL_USERNAME").expect("GMAIL_USERNAME environment variable is not set");

    let app_password =
        std::env::var("GMAIL_PWD").expect("GMAIL_APP_PASSWORD environment variable is not set");

    println!("Connecting to Gmail IMAP...");

    // Create a TLS connector.
    //
    // This validates Gmail's TLS certificate.
    let tls = TlsConnector::builder()
        .build()
        .expect("Failed to create TLS connector");

    // Establish a TLS connection to Gmail.
    let client = imap::connect((domain, port), domain, &tls)
        .expect("Failed to connect to Gmail IMAP server");

    println!("Connected successfully.");

    println!("Authenticating...");

    // Authenticate using:
    //
    // username     -> your Gmail address
    // app_password -> Google App Password
    let mut session = client
        .login(&username, &app_password)
        .map_err(|error| error.0)
        .expect("Failed to authenticate with Gmail");

    println!("Authentication successful!");

    let mailbox = session.examine("INBOX").expect("Failed to examine INBOX");

    println!("INBOX opened in read-only mode.");
    println!("Messages: {}", mailbox.exists);
    println!("Recent messages: {}", mailbox.recent);
    println!("Next UID: {}", mailbox.uid_next.unwrap_or(0));

    let messages = session
        .fetch("1:*", "(UID FLAGS ENVELOPE)")
        .expect("Failed to fetch message metadata");

    println!("\nMessages:");

    let mut emails: HashMap<String, i32> = HashMap::new();

    // let mut counter = 0;

    for message in messages.iter().rev().take(10000) {
        // let uid = message.uid.unwrap_or(0);

        // let is_seen = message
        //     .flags()
        //     .iter()
        //     .any(|flag| *flag == imap::types::Flag::Seen);

        let envelope = message.envelope().expect("Server did not return ENVELOPE");

        // let subject = envelope
        //     .subject
        //     .and_then(|subject| std::str::from_utf8(subject).ok())
        //     .unwrap_or("<no subject>");

        let from = envelope
            .from
            .as_ref()
            .and_then(|addresses| addresses.first())
            .map(format_address)
            .unwrap_or_else(|| "<unknown sender>".to_string());

        // let date = envelope
        //     .date
        //     .and_then(|date| std::str::from_utf8(date).ok())
        //     .unwrap_or("<unknown date>");

        if let Some(entry) = emails.get_mut(&from) {
            *entry += 1;
        } else {
            emails.insert(from, 1);
        }

        // println!("----------------------------------------");
        // println!("UID: {}", uid);
        // println!("From: {:?}", from);
        // println!("Subject: {}", subject);
        // println!("Date: {}", date);

        // println!("State: {}", if is_seen { "READ" } else { "UNREAD" });

        // if counter == 100 {
        //     break;
        // } else {
        //     counter += 1;
        // }

        // break;
    }

    let json = serde_json::to_string_pretty(&emails).expect("Failed to serialize emails");

    std::fs::write("emails.json", json).expect("Failed to write emails.json");

    session.logout().expect("Failed to logout cleanly");

    println!("Disconnected successfully.");

    Ok(())
}

fn format_address(address: &Address) -> String {
    let mailbox = address
        .mailbox
        .and_then(|mailbox| std::str::from_utf8(mailbox).ok())
        .unwrap_or("<unknown>");

    let host = address
        .host
        .and_then(|host| std::str::from_utf8(host).ok())
        .unwrap_or("<unknown>");

    format!("{mailbox}@{host}")
}
