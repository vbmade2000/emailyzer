use std::{
    collections::HashMap,
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};

use comfy_table::{ContentArrangement, Table, presets::UTF8_FULL};
use imap::{Client, Session, types::Fetch};
use imap_proto::types::{BodyStructure, ContentDisposition};
use native_tls::{TlsConnector, TlsStream};
use tokio::{sync::mpsc::channel, task::JoinHandle};
use tokio_imap::types::{Address, Envelope};
use tracing::{debug, info};

use crate::{
    ReceiverSortBy, ReceiversArgs, SenderSortBy, SendersArgs,
    database::{DatabaseLocation, DatabaseManager},
    types::{
        DATABASE_URL, DatabaseOperations, Email, GMAIL_IMAP_DOMAIN, GMAIL_IMAP_PORT, INBOX_MAILBOX,
        ReceiverStats, SENT_EMAILS_MAILBOX, SenderStats,
    },
};

/// Retrieves email credentials from env vars.
/// You can set it in current shell or .bashrc as below.
/// export GMAIL_USERNAME="your-gmail-username"
/// export GMAIL_PWD="your-gmail-password"
async fn get_credentials() -> anyhow::Result<(String, String)> {
    info!("Retrieving credentials from env var");
    // IMP: Do NOT hardcode your Gmail password or App Password in source code.
    Ok((
        std::env::var("GMAIL_USERNAME")?,
        std::env::var("GMAIL_PWD")?,
    ))
}

/// Create instance of TlsConnector to validate Gmail's TLS certificate
async fn get_tls_connector() -> anyhow::Result<TlsConnector> {
    info!("Building TLS Connector to validate Gmail certificate");
    Ok(TlsConnector::builder().build()?)
}

/// Read/write timeout applied to the underlying TCP socket. Without this, a blocking IMAP call
/// (`examine`, `uid_fetch`, etc.) can hang indefinitely if Gmail stops responding without
/// actively closing the connection, instead of surfacing as a retryable error.
const SOCKET_TIMEOUT: Duration = Duration::from_secs(60);

/// Create a client to connect to Gmail
/// 1. Resolve `domain`/`port` and open a `TcpStream` with read/write timeouts set, so a stalled
///    server can't hang the caller forever.
/// 2. Create an instance of `TlsConnector` and wrap the socket in TLS.
/// 3. Create an imap client from the TLS-wrapped stream.
async fn get_client(domain: &str, port: u16) -> anyhow::Result<Client<TlsStream<TcpStream>>> {
    info!("Creating a client");

    let address = (domain, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow::anyhow!("Could not resolve address for {}:{}", domain, port))?;

    let tcp_stream = TcpStream::connect_timeout(&address, SOCKET_TIMEOUT)?;
    tcp_stream.set_read_timeout(Some(SOCKET_TIMEOUT))?;
    tcp_stream.set_write_timeout(Some(SOCKET_TIMEOUT))?;

    let tls = get_tls_connector().await?;
    let tls_stream = tls.connect(domain, tcp_stream)?;

    Ok(Client::new(tls_stream))
}

/// Create a session instance
async fn get_session(
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
fn get_subject(envelope: &Envelope<'_>) -> String {
    envelope
        .subject
        .and_then(|subject| std::str::from_utf8(subject).ok())
        .unwrap_or("<no subject>")
        .to_string()
}

/// Extract sender/from field from envelope
fn get_sender(envelope: &Envelope<'_>) -> String {
    envelope
        .from
        .as_ref()
        .and_then(|addresses| addresses.first())
        .map(format_address)
        .unwrap_or_else(|| "<unknown sender>".to_string())
}

/// Extract receiver from envelope
fn get_receiver(envelope: &Envelope<'_>) -> String {
    envelope
        .to
        .as_ref()
        .and_then(|addresses| addresses.first())
        .map(format_address)
        .unwrap_or_else(|| "<unknown receiver>".to_string())
}

/// Extract datetime from envelope
fn get_datetime(envelope: &Envelope<'_>) -> String {
    envelope
        .date
        .and_then(|date| std::str::from_utf8(date).ok())
        .unwrap_or("<unknown date>")
        .to_string()
}

/// Extract is_seen flag from message
fn is_seen(message: &Fetch) -> bool {
    message.flags().contains(&imap::types::Flag::Seen)
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

/// Number of UIDs fetched per batch in `fetch_attachments_by_uid`. Keeps a single `uid_fetch`
/// request/response from becoming too large while still drastically cutting down the number of
/// network round-trips compared to fetching one UID at a time.
const ATTACHMENT_FETCH_BATCH_SIZE: usize = 1500;

/// Check if a `BODYSTRUCTURE` (or any of its subparts, for multipart/message messages) has an
/// attachment, by inspecting each part's `Content-Disposition`.
fn body_structure_has_attachment(body_structure: &BodyStructure) -> bool {
    let is_attachment = |disposition: &Option<ContentDisposition>| {
        disposition
            .as_ref()
            .map(|disposition| disposition.ty.eq_ignore_ascii_case("attachment"))
            .unwrap_or(false)
    };

    match body_structure {
        BodyStructure::Basic { common, .. } | BodyStructure::Text { common, .. } => {
            is_attachment(&common.disposition)
        }
        BodyStructure::Message { common, body, .. } => {
            is_attachment(&common.disposition) || body_structure_has_attachment(body)
        }
        BodyStructure::Multipart { common, bodies, .. } => {
            is_attachment(&common.disposition) || bodies.iter().any(body_structure_has_attachment)
        }
    }
}

/// Opens a fresh IMAP connection/session and `EXAMINE`s `mailbox`. Used to recover after a
/// session's underlying byte stream may have been left desynced (see `fetch_attachments_by_uid`),
/// since reusing a desynced session for further commands causes the `imap` crate to panic (via an
/// internal `assert_eq!` on response tags) rather than return a `Result`.
async fn reconnect_session(
    domain: &'static str,
    port: u16,
    username: &str,
    password: &str,
    mailbox: &str,
) -> anyhow::Result<Session<TlsStream<TcpStream>>> {
    let client = get_client(domain, port).await?;
    let mut session = get_session(username, password, client).await?;
    session.examine(mailbox)?;
    Ok(session)
}

/// Fetches `BODYSTRUCTURE` for `uids` in batches (via `session`, which must already be
/// `EXAMINE`d/`SELECT`ed on the relevant mailbox) and determines, for each UID, whether the
/// message has an attachment. `BODYSTRUCTURE` only transfers MIME metadata rather than full
/// message content, so this is fast even for mailboxes with large attachments.
///
/// If a batch's `BODYSTRUCTURE` fetch fails (e.g. the "Unable to parse status response" error the
/// `imap-proto` parser has been observed to raise on certain Gmail messages, which aborts parsing
/// of the whole batch response), every UID in that batch is recorded as `-1` (unknown) instead of
/// retrying with a slow per-UID `BODY.PEEK[]` fetch. Because a failed parse can leave `session`'s
/// underlying byte stream desynced (causing the `imap` crate to panic on later commands rather
/// than return an error), `session` is reconnected from scratch before continuing to subsequent
/// batches.
///
/// Never fails outright: any unrecoverable error (including reconnecting itself) is logged and
/// the affected UIDs are recorded as `-1` (unknown), since attachment detection is a best-effort
/// enrichment and shouldn't abort the fetch of otherwise-valid emails. If reconnecting fails,
/// remaining batches on this connection are skipped (also recorded as `-1`) rather than risking
/// further use of a possibly-desynced session.
#[allow(clippy::too_many_arguments)]
async fn fetch_attachments_by_uid(
    session: &mut Session<TlsStream<TcpStream>>,
    uids: &[u32],
    mailbox: &'static str,
    domain: &'static str,
    port: u16,
    username: &str,
    password: &str,
    connection_label: &str,
) -> HashMap<u32, i16> {
    let mut result = HashMap::with_capacity(uids.len());
    let total_batches = uids.len().div_ceil(ATTACHMENT_FETCH_BATCH_SIZE);

    for (batch_index, batch) in uids.chunks(ATTACHMENT_FETCH_BATCH_SIZE).enumerate() {
        if batch.is_empty() {
            continue;
        }

        info!(
            "[{}] Fetching attachment batch {}/{} ({} UIDs) from {}",
            connection_label,
            batch_index + 1,
            total_batches,
            batch.len(),
            mailbox
        );

        let uid_set = batch
            .iter()
            .map(|uid| uid.to_string())
            .collect::<Vec<_>>()
            .join(",");

        match session.uid_fetch(&uid_set, "BODYSTRUCTURE") {
            Ok(body_messages) => {
                for body_message in body_messages.iter() {
                    let uid = body_message.uid.unwrap_or(0);
                    let has_attachment = body_message
                        .bodystructure()
                        .map(body_structure_has_attachment)
                        .unwrap_or(false);
                    result.insert(uid, if has_attachment { 1 } else { 0 });
                }
            }
            Err(error) => {
                tracing::warn!(
                    "[{}] Failed to fetch BODYSTRUCTURE for UID batch ({} UIDs) from {}: {}. \
                     Reconnecting and marking this batch's attachment status as unknown (-1).",
                    connection_label,
                    batch.len(),
                    mailbox,
                    error
                );

                for &uid in batch {
                    result.insert(uid, -1);
                }

                match reconnect_session(domain, port, username, password, mailbox).await {
                    Ok(new_session) => {
                        *session = new_session;
                    }
                    Err(reconnect_error) => {
                        tracing::warn!(
                            "Failed to reconnect to {} after BODYSTRUCTURE failure: {}. \
                             Skipping remaining attachment batches on this connection.",
                            mailbox,
                            reconnect_error
                        );
                        break;
                    }
                }
            }
        }

        info!(
            "[{}] Fetched attachment batch {}/{} from {}",
            connection_label,
            batch_index + 1,
            total_batches,
            mailbox
        );
    }

    result
}

/// Number of parallel IMAP connections opened per mailbox to fetch `BODYSTRUCTURE` attachment
/// info concurrently. Each `BODYSTRUCTURE` batch is bound by Gmail's own per-message server-side
/// processing time (not payload size or client bandwidth), so a single connection processes
/// batches strictly one-at-a-time no matter how large they are. Splitting the UID range across
/// several concurrent connections lets Gmail work on multiple batches in parallel, cutting wall
/// time roughly by this factor.
const ATTACHMENT_FETCH_CONNECTIONS: usize = 4;

/// Opens `ATTACHMENT_FETCH_CONNECTIONS` separate IMAP connections/sessions to `mailbox`, splits
/// `uids` evenly across them, and fetches attachment info (see `fetch_attachments_by_uid`) for
/// each slice concurrently. This is purely an I/O-latency optimization: Gmail's `BODYSTRUCTURE`
/// response time per batch doesn't shrink with fewer UIDs, so running several batches at once
/// (each on its own connection) is what actually reduces wall-clock time.
///
/// If a connection can't be established/authenticated at all, that slice's UIDs are logged and
/// treated as "no attachment" rather than aborting the whole fetch.
async fn fetch_attachments_parallel(
    domain: &'static str,
    port: u16,
    username: String,
    password: String,
    mailbox: &'static str,
    uids: Vec<u32>,
) -> HashMap<u32, i16> {
    if uids.is_empty() {
        return HashMap::new();
    }

    let connections = ATTACHMENT_FETCH_CONNECTIONS.min(uids.len());
    let chunk_size = uids.len().div_ceil(connections);

    let mut handles = Vec::with_capacity(connections);
    for (connection_index, chunk) in uids.chunks(chunk_size).enumerate() {
        let chunk = chunk.to_vec();
        let username = username.clone();
        let password = password.clone();
        let connection_label = format!("conn {}/{}", connection_index + 1, connections);

        handles.push(tokio::spawn(async move {
            let client = get_client(domain, port).await?;
            let mut session = get_session(&username, &password, client).await?;
            session.examine(mailbox)?;

            let result = fetch_attachments_by_uid(
                &mut session,
                &chunk,
                mailbox,
                domain,
                port,
                &username,
                &password,
                &connection_label,
            )
            .await;

            session.logout()?;
            Ok::<HashMap<u32, i16>, anyhow::Error>(result)
        }));
    }

    let mut merged = HashMap::with_capacity(uids.len());
    for handle in handles {
        match handle.await {
            Ok(Ok(partial)) => merged.extend(partial),
            Ok(Err(error)) => {
                tracing::warn!(
                    "Attachment-fetch connection to {} failed: {}. Affected UIDs default to no attachment.",
                    mailbox,
                    error
                );
            }
            Err(join_error) => {
                tracing::warn!(
                    "Attachment-fetch task for {} panicked/was cancelled: {}. Affected UIDs default to no attachment.",
                    mailbox,
                    join_error
                );
            }
        }
    }

    merged
}

/// Spawns the database writer task.
///
/// A plain, IMAP-free task: it only owns the database connection and inserts whatever fully-formed
/// `Email`s (attachment detection already done by the sender) arrive over `db_receiver`. Exits
/// once the channel closes, i.e. once every fetch task sharing a clone of the sender has finished
/// and dropped its clone.
fn spawn_db_writer_task(
    mut db_receiver: tokio::sync::mpsc::Receiver<DatabaseOperations>,
) -> JoinHandle<anyhow::Result<()>> {
    tokio::spawn(async move {
        info!("Spawning database writer task");
        let db_manager = DatabaseManager::new(DatabaseLocation::File(DATABASE_URL)).await?;

        info!("Waiting for database write commands");
        // Exits once every fetch task's clone of the sender has been dropped (i.e. all mailbox
        // fetches are done), at which point `recv()` returns `None`.
        while let Some(DatabaseOperations::CreateEmailEntry(email)) = db_receiver.recv().await {
            debug!("DatabaseWriter: Creating email entry");
            db_manager.create_email_entry(email).await?;
        }

        info!("Stopping database writer task");

        Ok::<(), anyhow::Error>(())
    })
}

/// Spawns the task that fetches messages from `mailbox` and forwards them to the database writer
/// task.
///
/// Opens its own IMAP connection, `EXAMINE`s `mailbox`, and does a single bulk `UID FETCH` for
/// all message envelopes/flags/`BODYSTRUCTURE`s (attachment detection reads the already-fetched
/// `BODYSTRUCTURE` metadata, so no extra per-message round-trip is needed) before sending each
/// fully-formed `Email` over `db_sender`. Once done, drops its `db_sender` clone so the writer
/// task's channel can close once every fetch task sharing it has finished.
fn spawn_fetch_task(
    domain: &'static str,
    port: u16,
    username: String,
    password: String,
    mailbox: &'static str,
    db_sender: tokio::sync::mpsc::Sender<DatabaseOperations>,
) -> JoinHandle<anyhow::Result<()>> {
    info!("Spawning fetch task for {}", mailbox);

    tokio::spawn(async move {
        let fetch_started_at = Instant::now();

        let client = get_client(domain, port).await?;
        info!("Connected successfully");

        let mut session = get_session(&username, &password, client).await?;

        let _mailbox = session.examine(mailbox)?;

        let messages = session.uid_fetch("1:*", "(UID FLAGS ENVELOPE)")?;

        info!("Fetching {} messages from {}", messages.len(), mailbox);

        // Collect all UIDs so attachment detection can be done in batches instead of one
        // `BODY.PEEK[]` round-trip per email (which was the main sync bottleneck).
        info!(
            "Fetching attachment info for {} messages from {}",
            messages.len(),
            mailbox
        );
        let uids: Vec<u32> = messages.iter().map(|m| m.uid.unwrap_or(0)).collect();
        let attachments_by_uid = fetch_attachments_parallel(
            domain,
            port,
            username.clone(),
            password.clone(),
            mailbox,
            uids,
        )
        .await;
        info!("Attachment fetching done for {}", mailbox);

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
            let attachment = attachments_by_uid.get(&uid).copied().unwrap_or(-1);

            let email = Email {
                uid,
                subject: get_subject(envelope),
                sender: get_sender(envelope),
                read_status: is_seen,
                receiver: get_receiver(envelope),
                attachment,
                timestamp: get_datetime(envelope),
                body: "".to_string(),
                label: mailbox.to_lowercase(),
            };

            if db_sender
                .send(DatabaseOperations::CreateEmailEntry(email))
                .await
                .is_err()
            {
                // The receiver was dropped, which means the database writer task already
                // exited (most likely due to an error). The caller awaits that task separately
                // and surfaces its real underlying error instead of this generic one.
                anyhow::bail!("Database writer task exited unexpectedly");
            }
        }

        // Don't send an explicit Exit here: multiple fetch tasks share clones of `db_sender`,
        // and one finishing (e.g. a smaller mailbox) doesn't mean the others are done. Instead,
        // dropping this task's clone of `db_sender` (which happens automatically when the task
        // ends) lets the writer task's channel close naturally once *all* fetch tasks are done,
        // at which point `db_receiver.recv()` returns `None` and the writer task exits.

        session.logout()?;
        info!("Session disconnected successfully for {}.", mailbox);
        info!(
            "Finished fetching messages from {}. Stopping the relevant fetch task",
            mailbox
        );
        info!(
            "Total fetch time for {}: {:.2?}",
            mailbox,
            fetch_started_at.elapsed()
        );

        Ok::<(), anyhow::Error>(())
    })
}

/// Fetches emails from the provider and stores in database
pub async fn sync_emails() -> anyhow::Result<()> {
    info!("Syncing emails from {}", GMAIL_IMAP_DOMAIN);
    let (username, password) = get_credentials().await?;

    // Channel to send database operations to database writer task.
    // 100 is the buffer size. It means that the sender can send 100 messages before it blocks.
    // Our producer has network round-trip to Gmail and so it is relatively slow compare to consumer
    // so buffer size of 100 is more than enough.
    let (db_sender, db_receiver) = channel(500);
    let db_task = spawn_db_writer_task(db_receiver);
    let inbox_fetch_task = spawn_fetch_task(
        GMAIL_IMAP_DOMAIN,
        GMAIL_IMAP_PORT,
        username.clone(),
        password.clone(),
        INBOX_MAILBOX,
        db_sender.clone(),
    );
    let sent_emails_fetch_task = spawn_fetch_task(
        GMAIL_IMAP_DOMAIN,
        GMAIL_IMAP_PORT,
        username,
        password,
        SENT_EMAILS_MAILBOX,
        db_sender,
    );

    // Await both concurrently-running tasks. Prefer surfacing the writer task's error (if any)
    // since it's the true root cause; the fetch task's error in that scenario is just a generic
    // "channel closed" message caused by the writer task exiting first.
    let (inbox_fetch_result, db_result, sent_emails_fetch_result) =
        tokio::join!(inbox_fetch_task, db_task, sent_emails_fetch_task);
    db_result??;
    inbox_fetch_result??;
    sent_emails_fetch_result??;

    Ok(())
}

pub async fn get_sender_stats(sendersargs: SendersArgs) -> anyhow::Result<()> {
    let db_manager = DatabaseManager::new(DatabaseLocation::File(DATABASE_URL)).await?;

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
        let emails: Vec<Email> = db_manager.read_emails_from_database(INBOX_MAILBOX).await?;
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

            if email.attachment == 1 {
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
        Some(SenderSortBy::Emails) => rows.sort_by_key(|b| std::cmp::Reverse(b.total_emails)),
        Some(SenderSortBy::Sender) => rows.sort_by(|a, b| a.sender.cmp(&b.sender)),
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

pub async fn get_receiver_stats(receiversargs: ReceiversArgs) -> anyhow::Result<()> {
    let db_manager = DatabaseManager::new(DatabaseLocation::File(DATABASE_URL)).await?;

    // Prepare table for display
    let mut table = Table::new();
    table.set_header(vec![
        "Receiver",
        "Emails",
        "Read",
        "Unread",
        "Attachment",
        "No Attachment",
    ]);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.load_style(UTF8_FULL.with_rounded_corners());

    let preferred_receivers = receiversargs.receiver;

    let mut rows: Vec<ReceiverStats> = Vec::new();

    if receiversargs.refresh {
        info!("User has passed --refresh flag. Reading emails from database");
        let emails: Vec<Email> = db_manager
            .read_emails_from_database(&SENT_EMAILS_MAILBOX.to_lowercase())
            .await?;
        info!("Fetched {} emails from database", emails.len());

        let mut receivers: HashMap<String, ReceiverStats> = HashMap::new();

        // Count emails sent by each unique receiver
        for email in emails {
            let receiver_stat = receivers.entry(email.receiver.clone()).or_default();
            receiver_stat.receiver = email.receiver;

            receiver_stat.total_emails += 1;

            if email.read_status {
                receiver_stat.read_emails += 1;
            } else {
                receiver_stat.unread_emails += 1;
            }

            if email.attachment == 1 {
                receiver_stat.attachment_count += 1;
            } else {
                receiver_stat.no_attachment_count += 1;
            }
        }

        // Clear database table first to make fresh entries
        db_manager.delete_all_receiver_email_stats_entries().await?;

        // Save the stats in database because user has used --refresh flag. Also, print records on stdout
        for (receiver, stats) in receivers {
            db_manager
                .create_receiver_email_stats_entry(
                    receiver.clone(),
                    stats.total_emails,
                    stats.read_emails,
                    stats.unread_emails,
                    stats.attachment_count,
                    stats.no_attachment_count,
                )
                .await?;

            // We show only records from preferred senders if user has passed --sender flag
            if !preferred_receivers.is_empty() && !preferred_receivers.contains(&receiver) {
                continue;
            }

            rows.push(stats);
        }
    } else {
        info!(
            "User has skipped --refresh flag. Reading existing receiver email stats from database"
        );
        let receivers_stats = db_manager.read_receiver_email_stats().await?;

        if receivers_stats.is_empty() {
            info!(
                "No receiver email stats found in database. Please use --refresh flag to sync emails first"
            );
            return Ok(());
        }

        for receiver_stats in receivers_stats {
            // We show only records from preferred senders if user has passed --sender flag
            if !preferred_receivers.is_empty()
                && !preferred_receivers.contains(&receiver_stats.receiver)
            {
                continue;
            }
            rows.push(receiver_stats);
        }
    }

    // Sort the rows based on --sort-by flag, if provided. If --sort-by is absent but --top is
    // present, default to sorting by emails so "top N" has a well-defined meaning (highest
    // email counts first).
    match receiversargs.sort_by {
        Some(ReceiverSortBy::Emails) => rows.sort_by_key(|b| std::cmp::Reverse(b.total_emails)),
        Some(ReceiverSortBy::Receiver) => rows.sort_by(|a, b| a.receiver.cmp(&b.receiver)),
        None => {
            if receiversargs.top.is_some() {
                rows.sort_by_key(|b| std::cmp::Reverse(b.total_emails));
            }
        }
    }

    // Show only the top N rows (in whatever order was established above) if --top was passed.
    if let Some(top) = receiversargs.top {
        rows.truncate(top as usize);
    }

    for receiver_stat in rows {
        table.add_row(vec![
            receiver_stat.receiver,
            receiver_stat.total_emails.to_string(),
            receiver_stat.read_emails.to_string(),
            receiver_stat.unread_emails.to_string(),
            receiver_stat.attachment_count.to_string(),
            receiver_stat.no_attachment_count.to_string(),
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
