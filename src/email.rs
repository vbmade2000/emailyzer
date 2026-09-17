use std::{collections::HashMap, net::TcpStream};

use comfy_table::{ContentArrangement, Table, presets::UTF8_FULL};
use imap::{Client, Session, types::Fetch};
use mailparse::MailHeaderMap;
use native_tls::{TlsConnector, TlsStream};
use tokio_imap::types::{Address, Envelope};
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

/// Check if a parsed raw message (via `mailparse`) has an attachment by inspecting the
/// Content-Disposition header of the message itself and all of its subparts.
fn mail_has_attachment(parsed: &mailparse::ParsedMail) -> bool {
    let is_attachment = parsed
        .get_headers()
        .get_first_value("Content-Disposition")
        .map(|value| value.to_lowercase().starts_with("attachment"))
        .unwrap_or(false);

    is_attachment || parsed.subparts.iter().any(mail_has_attachment)
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

    let messages = session.uid_fetch("1:*", "(UID FLAGS ENVELOPE)")?;

    info!("Fetching {} messages", messages.len());

    for message in &messages {
        // Unique ID for every email. There is also sequence number but don't depend on it because it changes.
        let uid = message.uid.unwrap_or(0);

        // Extract fields
        let envelope = message.envelope().expect("Server did not return ENVELOPE");
        let is_seen = is_seen(message);
        let subject = get_subject(envelope).await;
        let sender = get_sender(envelope).await;
        let receiver = get_receiver(envelope);
        let datetime = get_datetime(envelope);

        // Fetch the raw message body (rather than relying on imap-proto's BODYSTRUCTURE parser,
        // which chokes on some of Gmail's responses) and parse it with `mailparse` to check for
        // attachments. This avoids the "Unable to parse status response" / broken pipe issues.
        let has_attachment = match session.uid_fetch(uid.to_string(), "BODY.PEEK[]") {
            Ok(body_messages) => body_messages
                .iter()
                .next()
                .and_then(|m| m.body())
                .and_then(|body| mailparse::parse_mail(body).ok())
                .map(|parsed| mail_has_attachment(&parsed))
                .unwrap_or(false),
            Err(error) => {
                tracing::warn!(
                    "Failed to fetch/parse body for UID {}: {}. Assuming no attachment.",
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

/// Returns a list of labels from the email account. Ex. INBOX, Drafts, Sent mails etc
async fn _get_mailboxes(
    session: &mut Session<TlsStream<TcpStream>>,
) -> anyhow::Result<Vec<String>> {
    let mailboxes = session.list(None, Some("*"))?;

    let mailbox_names = mailboxes.iter().map(|m| m.name().to_string()).collect();

    Ok(mailbox_names)
}
