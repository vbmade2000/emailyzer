use std::{collections::HashMap, net::TcpStream};

use comfy_table::{ContentArrangement, Table, presets::UTF8_FULL};
use imap::{Client, Session, types::Fetch};
use native_tls::{TlsConnector, TlsStream};
use tokio_imap::types::{Address, BodyContentCommon, BodyStructure, Envelope};
use tracing::info;

use crate::{
    SendersArgs, SortBy,
    database::{
        create_email_entry, create_or_open_db, create_sender_email_stats_entry,
        delete_all_sender_email_stats_entries, get_last_fetched_uid, read_emails_from_database,
        read_sender_email_stats,
    },
    types::Email,
};

/// Retrieves email credentials from env vars.
/// You can set it in current shell or .bashrc as below.
/// export GMAIL_USERNAME="your-gmail-username"
/// export GMAIL_PWD="your-gmail-password"
pub async fn get_credentials() -> anyhow::Result<(String, String)> {
    info!("Retrieving credentials from env var");
    // IMP: Do NOT hardcode your Gmail password or App Password in source code.
    Ok((
        std::env::var("GMAIL_USERNAME")?,
        std::env::var("GMAIL_PWD")?,
    ))
}

/// Create instance of TlsConnector to validate Gmail's TLS certificate
pub async fn get_tls_connector() -> anyhow::Result<TlsConnector> {
    info!("Building TLS Connector to validate Gmail certificate");
    Ok(TlsConnector::builder().build()?)
}

/// Create a client to connect to Gmail
/// 1. Create instance of TlsConnector
/// 2. Create an imap client
pub async fn get_client(domain: &str, port: u16) -> anyhow::Result<Client<TlsStream<TcpStream>>> {
    info!("Creating a client");
    let tls = get_tls_connector().await?;
    Ok(imap::connect((domain, port), domain, &tls)?)
}

/// Create a session instance
pub async fn get_session(
    username: &str,
    password: &str,
    client: Client<TlsStream<TcpStream>>,
) -> anyhow::Result<Session<TlsStream<TcpStream>>> {
    info!("Creating a session for interaction");
    let session = client.login(username, password).map_err(|error| error.0)?;
    info!("Authentication successful. Session established successfully");
    Ok(session)
}

/// Extract subject field from envelope
pub async fn get_subject(envelope: &Envelope<'_>) -> String {
    envelope
        .subject
        .and_then(|subject| std::str::from_utf8(subject).ok())
        .unwrap_or("<no subject>")
        .to_string()
}

// Extract sender/from field from envelope
pub async fn get_sender(envelope: &Envelope<'_>) -> String {
    envelope
        .from
        .as_ref()
        .and_then(|addresses| addresses.first())
        .map(format_address)
        .unwrap_or_else(|| "<unknown sender>".to_string())
}

/// Extract receiver from envelope
pub fn get_receiver(envelope: &Envelope<'_>) -> String {
    envelope
        .to
        .as_ref()
        .and_then(|addresses| addresses.first())
        .map(format_address)
        .unwrap_or_else(|| "<unknown receiver>".to_string())
}

/// Extract datetime from envelope
pub fn get_datetime(envelope: &Envelope<'_>) -> String {
    envelope
        .date
        .and_then(|date| std::str::from_utf8(date).ok())
        .unwrap_or("<unknown date>")
        .to_string()
}

/// Extract is_seen flag from message
pub fn is_seen(message: &Fetch) -> bool {
    message.flags().contains(&imap::types::Flag::Seen)
}

// Check if email has an attachment by inspecting its BODYSTRUCTURE
pub fn has_attachment(body_structure: &BodyStructure) -> bool {
    match body_structure {
        BodyStructure::Multipart { common, bodies, .. } => {
            is_attachment_disposition(common) || bodies.iter().any(has_attachment)
        }
        BodyStructure::Basic { common, .. } => is_attachment_disposition(common),
        BodyStructure::Text { common, .. } => is_attachment_disposition(common),
        BodyStructure::Message { common, body, .. } => {
            is_attachment_disposition(common) || has_attachment(body)
        }
    }
}

/// Check if a body part's content-disposition indicates it is an attachment
fn is_attachment_disposition(common: &BodyContentCommon) -> bool {
    common
        .disposition
        .as_ref()
        .is_some_and(|disposition| disposition.ty.eq_ignore_ascii_case("attachment"))
}

/// Format email address
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

pub async fn sync_emails() -> anyhow::Result<()> {
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

    for message in messages.iter().take(100) {
        // Unique ID for every email. There is also sequence number but don't depend on it because it changes.
        let uid = message.uid.unwrap_or(0);

        // Extract fields
        let envelope = message.envelope().expect("Server did not return ENVELOPE");
        let is_seen = is_seen(message);
        let subject = get_subject(envelope).await;
        let sender = get_sender(envelope).await;
        let receiver = get_receiver(envelope);
        let datetime = get_datetime(envelope);

        // TODO: Try the way mentioned at the end of this file.
        // Fetch BODYSTRUCTURE per-message (rather than batching it with the rest) so that a
        // single message with a structure the parser chokes on (a known imap-proto limitation
        // with some of Gmail's BODYSTRUCTURE responses) doesn't fail the entire batch fetch.
        let has_attachment = match session.uid_fetch(uid.to_string(), "BODYSTRUCTURE") {
            Ok(bs_messages) => bs_messages
                .iter()
                .next()
                .and_then(|m| m.bodystructure())
                .map(has_attachment)
                .unwrap_or(false),
            Err(error) => {
                tracing::warn!(
                    "Failed to fetch/parse BODYSTRUCTURE for UID {}: {}. Assuming no attachment.",
                    uid,
                    error
                );
                false
            }
        };

        let email = Email {
            uid,
            subject,
            sender,
            read_status: is_seen,
            receiver,
            attachment: has_attachment,
            timestamp: datetime,
            body: "".to_string(),
        };

        create_email_entry(&conn, email).await?;
    }

    session.logout()?;

    info!("Session disconnected successfully.");

    Ok(())
}

pub async fn get_sender_stats(sendersargs: SendersArgs) -> anyhow::Result<()> {
    let conn = create_or_open_db("emailyzer.db").await?;

    // Prepare table for display
    let mut table = Table::new();
    table.set_header(vec![
        "Sender",
        "Emails",
        "Read",
        "Unread",
        "Attachment",
        "No Attachment",
    ]);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.load_style(UTF8_FULL.with_rounded_corners());

    let preferred_senders = sendersargs.sender;

    // (sender, emails, read-emails, unread-emails, email_with_attachment, email_without_attachment)
    let mut rows: Vec<(String, u32, u32, u32, u32, u32)> = Vec::new();

    if sendersargs.refresh {
        info!("User has passed --refresh flag. Reading emails from database");
        let emails: Vec<Email> = read_emails_from_database(&conn).await?;
        info!("Fetched {} emails from database", emails.len());

        // (sender, emails, read-emails, unread-emails, email_with_attachment, email_without_attachment)
        let mut senders: HashMap<String, (u32, u32, u32, u32, u32)> = HashMap::new();

        // Count emails sent by each unique sender
        for email in emails {
            let count = senders.entry(email.sender).or_insert((0, 0, 0, 0, 0));
            count.0 += 1;
            if email.read_status {
                count.1 += 1;
            } else {
                count.2 += 1;
            }

            if email.attachment {
                count.3 += 1;
            } else {
                count.4 += 1;
            }
        }

        // Clear database table first to make fresh entries
        delete_all_sender_email_stats_entries(&conn).await?;

        // Save the stats in database because user has used --refresh flag. Also, print records on stdout
        for (sender, count) in senders {
            create_sender_email_stats_entry(
                &conn,
                sender.clone(),
                count.0,
                count.1,
                count.2,
                count.3,
                count.4,
            )
            .await?;
            // We show only records from preferred senders if user has passed --sender flag
            if !preferred_senders.is_empty() && !preferred_senders.contains(&sender) {
                continue;
            }
            rows.push((sender, count.0, count.1, count.2, count.3, count.4));
        }
    } else {
        info!("User has skipped --refresh flag. Reading existing sender email stats from database");
        let senders_stats = read_sender_email_stats(&conn).await?;

        if senders_stats.is_empty() {
            info!(
                "No sender email stats found in database. Please use --refresh flag to sync emails first"
            );
            return Ok(());
        }

        for (sender, count, read, unread, with_attachment, without_attachment) in senders_stats {
            // We show only records from preferred senders if user has passed --sender flag
            if !preferred_senders.is_empty() && !preferred_senders.contains(&sender) {
                continue;
            }
            rows.push((
                sender,
                count,
                read,
                unread,
                with_attachment,
                without_attachment,
            ));
        }
    }

    // Sort the rows based on --sort-by flag, if provided
    match sendersargs.sort_by {
        Some(SortBy::Emails) => rows.sort_by_key(|b| std::cmp::Reverse(b.1)),
        Some(SortBy::Sender) => rows.sort_by(|a, b| a.0.cmp(&b.0)),
        None => {}
    }

    // Show top N senders by no of emails if user has passed --top flag
    if let Some(top) = sendersargs.top {
        rows.sort_by_key(|b| std::cmp::Reverse(b.1));
        rows.truncate(top as usize);
    }

    for (sender, count, read, unread, with_attachment, without_attachment) in rows {
        table.add_row(vec![
            sender,
            count.to_string(),
            read.to_string(),
            unread.to_string(),
            with_attachment.to_string(),
            without_attachment.to_string(),
        ]);
    }

    println!("{table}");

    Ok(())
}

// let messages = session.uid_fetch(
//     uid.to_string(),
//     "(UID BODY.PEEK[])"
// )?;

// for message in messages.iter() {
//     if let Some(body) = message.body() {
//         let parsed = mailparse::parse_mail(body)?;

//         let has_attachment = parsed.subparts.iter().any(|part| {
//             part.get_headers()
//                 .get_first_value("Content-Disposition")
//                 .map(|v| v.to_lowercase().starts_with("attachment"))
//                 .unwrap_or(false)
//         });

//         println!("Has attachment: {}", has_attachment);
//     }
// }
