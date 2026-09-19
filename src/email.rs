use std::{collections::HashMap, net::TcpStream};

use comfy_table::{ContentArrangement, Table, presets::UTF8_FULL};
use imap::{Client, Session, types::Fetch};
use mailparse::MailHeaderMap;
use native_tls::{TlsConnector, TlsStream};
use tokio::{sync::mpsc::channel, task::JoinHandle};
use tokio_imap::types::{Address, Envelope};
use tracing::{debug, info};

use crate::{
    SendersArgs, SortBy,
    database::DatabaseManager,
    types::{DATABASE_URL, DatabaseOperations, Email, SenderStats},
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
pub fn get_subject(envelope: &Envelope<'_>) -> String {
    envelope
        .subject
        .and_then(|subject| std::str::from_utf8(subject).ok())
        .unwrap_or("<no subject>")
        .to_string()
}

// Extract sender/from field from envelope
pub fn get_sender(envelope: &Envelope<'_>) -> String {
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

/// Fetches emails from the provider and stores in database
pub async fn sync_emails() -> anyhow::Result<()> {
    // Channel to send database operations to database writer task.
    // 100 is the buffer size. It means that the sender can send 100 messages before it blocks.
    // Our producer has network round-trip to Gmail and so it is relatively slow compare to consumer
    // so buffer size of 100 is more than enough.
    let (db_sender, mut db_receiver) = channel(100);
    let db_task: JoinHandle<anyhow::Result<()>> = tokio::spawn(async move {
        info!("Spawning database writer task");
        let db_manager = DatabaseManager::new(DATABASE_URL).await?;

        info!("Waiting for database write commands");
        while let Some(command) = db_receiver.recv().await {
            match command {
                DatabaseOperations::CreateEmailEntry(email) => {
                    debug!("DatabaseWriter: Creating email entry in database");
                    db_manager.create_email_entry(email).await?;
                }
                DatabaseOperations::Exit => {
                    info!("DatabaseWriter: Exit command received");
                    break;
                }
            }
        }

        info!("Stopping database writer task");

        Ok::<(), anyhow::Error>(())
    });

    // Gmail IMAP server.
    let domain = "imap.gmail.com";
    let port = 993;

    let (username, password) = get_credentials().await?;

    let client = get_client(domain, port).await?;
    info!("Connected successfully.");

    info!("Authenticating...");
    let mut session = get_session(&username, &password, client).await?;

    let _mailbox = session.examine("INBOX")?;

    let messages = session.uid_fetch("1:*", "(UID FLAGS ENVELOPE)")?;

    info!("Fetching {} messages", messages.len());

    for message in &messages {
        // Unique ID for every email. There is also sequence number but don't depend on it because it changes.
        let uid = message.uid.unwrap_or(0);

        // Extract fields
        let envelope = message.envelope();

        // This is an edge case. There may be an email for which the envelope would not be retreieved.
        if envelope.is_none() {
            tracing::warn!("Envelope is None for UID {}, skipping", uid);
            continue;
        }

        let envelope = envelope.as_ref().unwrap();
        let is_seen = is_seen(message);
        let subject = get_subject(envelope);
        let sender = get_sender(envelope);
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

        if db_sender
            .send(DatabaseOperations::CreateEmailEntry(email))
            .await
            .is_err()
        {
            // The receiver was dropped, which means the database writer task already exited
            // (most likely due to an error). Await it now to surface the real underlying error
            // instead of the generic "channel closed" message.
            db_task.await??;
            anyhow::bail!("Database writer task exited unexpectedly");
        }
    }

    // Only send Exit if the writer task is still alive; ignore error here since the task may
    // have already stopped, in which case db_task.await below will surface the real error.
    let _ = db_sender.send(DatabaseOperations::Exit).await;

    // Ensure the database writer task has finished flushing all writes before returning,
    // and propagate any error it encountered.
    db_task.await??;

    session.logout()?;

    info!("Session disconnected successfully.");

    Ok(())
}

pub async fn get_sender_stats(sendersargs: SendersArgs) -> anyhow::Result<()> {
    let db_manager = DatabaseManager::new(DATABASE_URL).await?;

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

    let mut rows: Vec<SenderStats> = Vec::new();

    if sendersargs.refresh {
        info!("User has passed --refresh flag. Reading emails from database");
        let emails: Vec<Email> = db_manager.read_emails_from_database().await?;
        info!("Fetched {} emails from database", emails.len());

        let mut senders: HashMap<String, SenderStats> = HashMap::new();

        // Count emails sent by each unique sender
        for email in emails {
            let sender_stat = senders.entry(email.sender.clone()).or_default();
            sender_stat.sender = email.sender;

            sender_stat.total_emails += 1;

            if email.read_status {
                sender_stat.read_emails += 1;
            } else {
                sender_stat.unread_emails += 1;
            }

            if email.attachment {
                sender_stat.attachment_count += 1;
            } else {
                sender_stat.no_attachment_count += 1;
            }
        }

        // Clear database table first to make fresh entries
        db_manager.delete_all_sender_email_stats_entries().await?;

        // Save the stats in database because user has used --refresh flag. Also, print records on stdout
        for (sender, stats) in senders {
            db_manager
                .create_sender_email_stats_entry(
                    sender.clone(),
                    stats.total_emails,
                    stats.read_emails,
                    stats.unread_emails,
                    stats.attachment_count,
                    stats.no_attachment_count,
                )
                .await?;

            // We show only records from preferred senders if user has passed --sender flag
            if !preferred_senders.is_empty() && !preferred_senders.contains(&sender) {
                continue;
            }

            rows.push(stats);
        }
    } else {
        info!("User has skipped --refresh flag. Reading existing sender email stats from database");
        let senders_stats = db_manager.read_sender_email_stats().await?;

        if senders_stats.is_empty() {
            info!(
                "No sender email stats found in database. Please use --refresh flag to sync emails first"
            );
            return Ok(());
        }

        for sender_stat in senders_stats {
            // We show only records from preferred senders if user has passed --sender flag
            if !preferred_senders.is_empty() && !preferred_senders.contains(&sender_stat.sender) {
                continue;
            }
            rows.push(sender_stat);
        }
    }

    // Sort the rows based on --sort-by flag, if provided. If --sort-by is absent but --top is
    // present, default to sorting by emails so "top N" has a well-defined meaning (highest
    // email counts first).
    match sendersargs.sort_by {
        Some(SortBy::Emails) => rows.sort_by_key(|b| std::cmp::Reverse(b.total_emails)),
        Some(SortBy::Sender) => rows.sort_by(|a, b| a.sender.cmp(&b.sender)),
        None => {
            if sendersargs.top.is_some() {
                rows.sort_by_key(|b| std::cmp::Reverse(b.total_emails));
            }
        }
    }

    // Show only the top N rows (in whatever order was established above) if --top was passed.
    if let Some(top) = sendersargs.top {
        rows.truncate(top as usize);
    }

    for sender_stat in rows {
        table.add_row(vec![
            sender_stat.sender,
            sender_stat.total_emails.to_string(),
            sender_stat.read_emails.to_string(),
            sender_stat.unread_emails.to_string(),
            sender_stat.attachment_count.to_string(),
            sender_stat.no_attachment_count.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_envelope<'a>(
        subject: Option<&'a [u8]>,
        from: Option<Vec<Address<'a>>>,
        to: Option<Vec<Address<'a>>>,
        date: Option<&'a [u8]>,
    ) -> Envelope<'a> {
        Envelope {
            date,
            subject,
            from,
            sender: None,
            reply_to: None,
            to,
            cc: None,
            bcc: None,
            in_reply_to: None,
            message_id: None,
        }
    }

    fn make_address<'a>(mailbox: Option<&'a [u8]>, host: Option<&'a [u8]>) -> Address<'a> {
        Address {
            name: None,
            adl: None,
            mailbox,
            host,
        }
    }

    // get_subject -----------------------------------------------------------

    #[test]
    fn get_subject_returns_subject_when_present() {
        let envelope = make_envelope(Some(b"Hello world"), None, None, None);
        assert_eq!(get_subject(&envelope), "Hello world");
    }

    #[test]
    fn get_subject_returns_placeholder_when_missing() {
        let envelope = make_envelope(None, None, None, None);
        assert_eq!(get_subject(&envelope), "<no subject>");
    }

    #[test]
    fn get_subject_returns_placeholder_on_invalid_utf8() {
        let envelope = make_envelope(Some(&[0xff, 0xfe]), None, None, None);
        assert_eq!(get_subject(&envelope), "<no subject>");
    }

    #[test]
    fn get_subject_returns_empty_string_when_subject_is_empty() {
        let envelope = make_envelope(Some(b""), None, None, None);
        assert_eq!(get_subject(&envelope), "");
    }

    // get_sender ------------------------------------------------------------

    #[test]
    fn get_sender_returns_formatted_address() {
        let envelope = make_envelope(
            None,
            Some(vec![make_address(Some(b"malhar"), Some(b"example.com"))]),
            None,
            None,
        );
        assert_eq!(get_sender(&envelope), "malhar@example.com");
    }

    #[test]
    fn get_sender_uses_first_address_only() {
        let envelope = make_envelope(
            None,
            Some(vec![
                make_address(Some(b"malhar"), Some(b"example.com")),
                make_address(Some(b"nimesh"), Some(b"example.com")),
            ]),
            None,
            None,
        );
        assert_eq!(get_sender(&envelope), "malhar@example.com");
    }

    #[test]
    fn get_sender_returns_placeholder_when_missing() {
        let envelope = make_envelope(None, None, None, None);
        assert_eq!(get_sender(&envelope), "<unknown sender>");
    }

    #[test]
    fn get_sender_returns_placeholder_when_from_is_empty() {
        let envelope = make_envelope(None, Some(vec![]), None, None);
        assert_eq!(get_sender(&envelope), "<unknown sender>");
    }

    #[test]
    fn get_sender_handles_missing_mailbox_and_host() {
        let envelope = make_envelope(None, Some(vec![make_address(None, None)]), None, None);
        assert_eq!(get_sender(&envelope), "<unknown>@<unknown>");
    }

    #[test]
    fn get_sender_handles_invalid_utf8() {
        let envelope = make_envelope(
            None,
            Some(vec![make_address(Some(&[0xff]), Some(&[0xfe]))]),
            None,
            None,
        );
        assert_eq!(get_sender(&envelope), "<unknown>@<unknown>");
    }

    // get_receiver ----------------------------------------------------------

    #[test]
    fn get_receiver_returns_formatted_address() {
        let envelope = make_envelope(
            None,
            None,
            Some(vec![make_address(Some(b"malhar"), Some(b"example.com"))]),
            None,
        );
        assert_eq!(get_receiver(&envelope), "malhar@example.com");
    }

    #[test]
    fn get_receiver_uses_first_address_only() {
        let envelope = make_envelope(
            None,
            None,
            Some(vec![
                make_address(Some(b"nimesh"), Some(b"example.com")),
                make_address(Some(b"adi"), Some(b"example.com")),
            ]),
            None,
        );
        assert_eq!(get_receiver(&envelope), "nimesh@example.com");
    }

    #[test]
    fn get_receiver_returns_placeholder_when_missing() {
        let envelope = make_envelope(None, None, None, None);
        assert_eq!(get_receiver(&envelope), "<unknown receiver>");
    }

    #[test]
    fn get_receiver_returns_placeholder_when_to_is_empty() {
        let envelope = make_envelope(None, None, Some(vec![]), None);
        assert_eq!(get_receiver(&envelope), "<unknown receiver>");
    }

    #[test]
    fn get_receiver_handles_missing_mailbox_and_host() {
        let envelope = make_envelope(None, None, Some(vec![make_address(None, None)]), None);
        assert_eq!(get_receiver(&envelope), "<unknown>@<unknown>");
    }

    // get_datetime ----------------------------------------------------------

    #[test]
    fn get_datetime_returns_date_when_present() {
        let envelope = make_envelope(None, None, None, Some(b"Mon, 1 Jan 2024 00:00:00 +0000"));
        assert_eq!(get_datetime(&envelope), "Mon, 1 Jan 2024 00:00:00 +0000");
    }

    #[test]
    fn get_datetime_returns_placeholder_when_missing() {
        let envelope = make_envelope(None, None, None, None);
        assert_eq!(get_datetime(&envelope), "<unknown date>");
    }

    #[test]
    fn get_datetime_returns_placeholder_on_invalid_utf8() {
        let envelope = make_envelope(None, None, None, Some(&[0xff, 0xfe]));
        assert_eq!(get_datetime(&envelope), "<unknown date>");
    }

    // format_address --------------------------------------------------------

    #[test]
    fn format_address_combines_mailbox_and_host() {
        let address = make_address(Some(b"malhar"), Some(b"example.com"));
        assert_eq!(format_address(&address), "malhar@example.com");
    }

    #[test]
    fn format_address_uses_unknown_for_missing_parts() {
        let address = make_address(None, None);
        assert_eq!(format_address(&address), "<unknown>@<unknown>");
    }

    #[test]
    fn format_address_uses_unknown_for_invalid_utf8() {
        let address = make_address(Some(&[0xff]), Some(&[0xfe]));
        assert_eq!(format_address(&address), "<unknown>@<unknown>");
    }

    #[test]
    fn format_address_keeps_valid_part_when_other_is_invalid() {
        let address = make_address(Some(b"malhar"), Some(&[0xfe]));
        assert_eq!(format_address(&address), "malhar@<unknown>");

        let address = make_address(Some(&[0xff]), Some(b"example.com"));
        assert_eq!(format_address(&address), "<unknown>@example.com");
    }

    // mail_has_attachment ---------------------------------------------------
    // (private helper, reachable because tests are a child module of `email`)

    #[test]
    fn mail_without_attachment_returns_false() {
        let raw = b"From: malhar@example.com\r\n\
            To: malhar@example.com\r\n\
            Subject: Hello\r\n\
            Content-Type: text/plain\r\n\
            \r\n\
            Hello world\r\n";
        let parsed = mailparse::parse_mail(raw).expect("test email should parse");
        assert!(!mail_has_attachment(&parsed));
    }

    #[test]
    fn mail_with_top_level_attachment_disposition_returns_true() {
        let raw = b"From: malhar@example.com\r\n\
            Content-Disposition: attachment; filename=\"test.txt\"\r\n\
            Content-Type: text/plain\r\n\
            \r\n\
            Hello\r\n";
        let parsed = mailparse::parse_mail(raw).expect("test email should parse");
        assert!(mail_has_attachment(&parsed));
    }

    #[test]
    fn mail_with_attachment_subpart_returns_true() {
        let raw = b"From: malhar@example.com\r\n\
            To: malhar@example.com\r\n\
            Subject: files\r\n\
            MIME-Version: 1.0\r\n\
            Content-Type: multipart/mixed; boundary=\"BOUNDARY\"\r\n\
            \r\n\
            --BOUNDARY\r\n\
            Content-Type: text/plain\r\n\
            \r\n\
            Hello\r\n\
            \r\n\
            --BOUNDARY\r\n\
            Content-Type: application/octet-stream\r\n\
            Content-Disposition: attachment; filename=\"test.txt\"\r\n\
            Content-Transfer-Encoding: base64\r\n\
            \r\n\
            aGVsbG8=\r\n\
            --BOUNDARY--\r\n";
        let parsed = mailparse::parse_mail(raw).expect("test email should parse");
        assert!(mail_has_attachment(&parsed));
    }

    #[test]
    fn mail_with_nested_attachment_returns_true() {
        let raw = b"MIME-Version: 1.0\r\n\
            Content-Type: multipart/mixed; boundary=\"OUTER\"\r\n\
            \r\n\
            --OUTER\r\n\
            Content-Type: multipart/alternative; boundary=\"INNER\"\r\n\
            \r\n\
            --INNER\r\n\
            Content-Type: text/plain\r\n\
            \r\n\
            Hello\r\n\
            \r\n\
            --INNER--\r\n\
            \r\n\
            --OUTER\r\n\
            Content-Type: application/pdf\r\n\
            Content-Disposition: attachment; filename=\"doc.pdf\"\r\n\
            \r\n\
            fake-bytes\r\n\
            --OUTER--\r\n";
        let parsed = mailparse::parse_mail(raw).expect("test email should parse");
        assert!(mail_has_attachment(&parsed));
    }

    #[test]
    fn mail_with_inline_disposition_returns_false() {
        let raw = b"From: malhar@example.com\r\n\
            To: malhar@example.com\r\n\
            MIME-Version: 1.0\r\n\
            Content-Type: multipart/mixed; boundary=\"BOUNDARY\"\r\n\
            \r\n\
            --BOUNDARY\r\n\
            Content-Type: text/plain\r\n\
            \r\n\
            Hello\r\n\
            \r\n\
            --BOUNDARY\r\n\
            Content-Type: image/png\r\n\
            Content-Disposition: inline; filename=\"image.png\"\r\n\
            \r\n\
            fake-bytes\r\n\
            --BOUNDARY--\r\n";
        let parsed = mailparse::parse_mail(raw).expect("test email should parse");
        assert!(!mail_has_attachment(&parsed));
    }

    #[test]
    fn mail_attachment_detection_is_case_insensitive() {
        let raw = b"From: malhar@example.com\r\n\
            Content-Type: application/octet-stream\r\n\
            Content-Disposition: ATTACHMENT; filename=\"test.txt\"\r\n\
            \r\n\
            data\r\n";
        let parsed = mailparse::parse_mail(raw).expect("test email should parse");
        assert!(mail_has_attachment(&parsed));
    }

    // get_credentials -------------------------------------------------------
    // Env vars are process-global, so guard these tests with a mutex and
    // restore the original values afterwards.

    static ENV_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_saved_env() -> (
        std::sync::MutexGuard<'static, ()>,
        Option<String>,
        Option<String>,
    ) {
        let guard = ENV_GUARD.lock().unwrap();
        let username = std::env::var("GMAIL_USERNAME").ok();
        let password = std::env::var("GMAIL_PWD").ok();
        (guard, username, password)
    }

    fn restore_env(username: Option<String>, password: Option<String>) {
        unsafe {
            match username {
                Some(value) => std::env::set_var("GMAIL_USERNAME", value),
                None => std::env::remove_var("GMAIL_USERNAME"),
            }
            match password {
                Some(value) => std::env::set_var("GMAIL_PWD", value),
                None => std::env::remove_var("GMAIL_PWD"),
            }
        }
    }

    fn block_on_credentials() -> anyhow::Result<(String, String)> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime should build")
            .block_on(get_credentials())
    }

    #[test]
    fn get_credentials_returns_values_from_env() {
        let (_guard, saved_username, saved_password) = with_saved_env();
        unsafe {
            std::env::set_var("GMAIL_USERNAME", "malhar@example.com");
            std::env::set_var("GMAIL_PWD", "secret");
        }

        let result = block_on_credentials();

        restore_env(saved_username, saved_password);
        assert_eq!(
            result.expect("credentials should be returned"),
            ("malhar@example.com".to_string(), "secret".to_string())
        );
    }

    #[test]
    fn get_credentials_fails_when_username_is_missing() {
        let (_guard, saved_username, saved_password) = with_saved_env();
        unsafe {
            std::env::remove_var("GMAIL_USERNAME");
            std::env::set_var("GMAIL_PWD", "secret");
        }

        let result = block_on_credentials();

        restore_env(saved_username, saved_password);
        assert!(result.is_err());
    }

    #[test]
    fn get_credentials_fails_when_password_is_missing() {
        let (_guard, saved_username, saved_password) = with_saved_env();
        unsafe {
            std::env::set_var("GMAIL_USERNAME", "malhar@example.com");
            std::env::remove_var("GMAIL_PWD");
        }

        let result = block_on_credentials();

        restore_env(saved_username, saved_password);
        assert!(result.is_err());
    }

    // get_tls_connector -----------------------------------------------------

    #[tokio::test]
    async fn get_tls_connector_builds_successfully() {
        assert!(get_tls_connector().await.is_ok());
    }

    // get_client ------------------------------------------------------------
    // `.invalid` (RFC 2606) never resolves, so this exercises the error path
    // without touching the real Gmail servers.

    #[tokio::test]
    async fn get_client_returns_error_for_unresolvable_host() {
        let result = get_client("invalid.invalid", 993).await;
        assert!(result.is_err());
    }

    // NOTE: `is_seen`, `sync_emails`, `get_sender_stats`, and `_get_mailboxes`
    // are intentionally not unit-tested here. `is_seen` takes an
    // `imap::types::Fetch` whose flag storage is `pub(crate)` to the `imap`
    // crate, so it cannot be constructed from outside that crate. The others
    // require a live IMAP connection and/or a SQLite database file.
}
