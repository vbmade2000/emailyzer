use std::{collections::HashMap, net::TcpStream, time::Instant};

use imap::Session;
use native_tls::TlsStream;
use tokio::{
    sync::{mpsc::channel, watch},
    task::JoinHandle,
};
use tracing::{debug, info};

use crate::{
    SyncArgs,
    database::{DATABASE_URL, DatabaseLocation, DatabaseManager, DatabaseOperations},
    email::{
        imap_client::{get_client, get_session, reconnect_session},
        parsing::{
            body_structure_has_attachment, get_datetime, get_receiver, get_sender, get_subject,
            is_seen,
        },
        types::Email,
    },
    password_store::get_password,
};

/// Shared signal used to implement graceful shutdown. All long-running tasks spawned by
/// `sync_emails` (the database writer task and the per-mailbox fetch/attachment tasks) hold a
/// clone of the receiver and check it after finishing the record (email/batch/etc.) they're
/// currently working on. If it has flipped to `true`, the task stops picking up new work and
/// exits cleanly instead of being killed mid-record.
pub(crate) type ShutdownSignal = watch::Receiver<bool>;

/// Returns `true` once a shutdown has been requested (e.g. via Ctrl+C).
fn shutdown_requested(shutdown: &ShutdownSignal) -> bool {
    *shutdown.borrow()
}

/// Waits for either Ctrl+C (SIGINT) or, on Unix, SIGTERM (the signal `systemd`, Docker, and
/// Kubernetes send for a normal "please stop" request before escalating to an unmaskable
/// SIGKILL). SIGKILL itself can never be caught by any process, so there's nothing to add for it
/// here; SIGTERM is the catchable signal process managers use to ask for a graceful stop.
async fn wait_for_shutdown_request() {
    #[cfg(unix)]
    {
        let mut sigterm = match tokio::signal::unix::signal(
            tokio::signal::unix::SignalKind::terminate(),
        ) {
            Ok(sigterm) => sigterm,
            Err(error) => {
                tracing::warn!(
                    "Failed to install SIGTERM handler: {}. Only Ctrl+C will trigger graceful shutdown.",
                    error
                );
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Spawns a task that waits for a shutdown request (Ctrl+C, or SIGTERM on Unix) and, once
/// received, flips the shared shutdown flag so every task sharing `shutdown_tx` finishes its
/// current record and exits.
///
/// A second shutdown request (received any time after the first) force-quits the whole process
/// immediately via `std::process::exit`, bypassing the graceful finish-current-record logic, for
/// cases where the caller doesn't want to wait (e.g. a stuck network call).
fn spawn_shutdown_listener(shutdown_tx: watch::Sender<bool>) -> JoinHandle<()> {
    tokio::spawn(async move {
        wait_for_shutdown_request().await;
        info!(
            "Shutdown signal received. Finishing in-flight records before exiting. Press Ctrl+C again to force quit."
        );
        let _ = shutdown_tx.send(true);

        wait_for_shutdown_request().await;
        info!("Second shutdown signal received. Force quitting immediately.");
        std::process::exit(130);
    })
}

/// Number of UIDs fetched per batch in `fetch_attachments_by_uid`. Keeps a single `uid_fetch`
/// request/response from becoming too large while still drastically cutting down the number of
/// network round-trips compared to fetching one UID at a time.
const ATTACHMENT_FETCH_BATCH_SIZE: usize = 1500;

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
    mailbox: &str,
    domain: &str,
    port: u16,
    username: &str,
    password: &str,
    connection_label: &str,
    shutdown: &ShutdownSignal,
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

        if shutdown_requested(shutdown) {
            info!(
                "[{}] Shutdown requested by user: finished current attachment batch for {}, stopping before remaining batches",
                connection_label, mailbox
            );
            break;
        }
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

/// Buffer size for the channel between fetch tasks and the database writer task: the sender can
/// send this many messages before it blocks. The producer side has a network round-trip to Gmail
/// per message and so is relatively slow compared to the consumer (a local SQLite insert), so this
/// buffer size is more than enough to avoid backpressure in practice.
const DB_CHANNEL_BUFFER_SIZE: usize = 500;

/// Opens `ATTACHMENT_FETCH_CONNECTIONS` separate IMAP connections/sessions to `mailbox`, splits
/// `uids` evenly across them, and fetches attachment info (see `fetch_attachments_by_uid`) for
/// each slice concurrently. This is purely an I/O-latency optimization: Gmail's `BODYSTRUCTURE`
/// response time per batch doesn't shrink with fewer UIDs, so running several batches at once
/// (each on its own connection) is what actually reduces wall-clock time.
///
/// If a connection can't be established/authenticated at all, that slice's UIDs are logged and
/// treated as "no attachment" rather than aborting the whole fetch.
async fn fetch_attachments_parallel(
    domain: String,
    port: u16,
    username: String,
    password: String,
    mailbox: String,
    uids: Vec<u32>,
    shutdown: ShutdownSignal,
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
        let domain = domain.clone();
        let mailbox = mailbox.clone();
        let connection_label = format!("conn {}/{}", connection_index + 1, connections);
        let shutdown = shutdown.clone();

        handles.push(tokio::spawn(async move {
            let client = get_client(&domain, port).await?;
            let mut session = get_session(&username, &password, client).await?;
            session.examine(&mailbox)?;

            let result = fetch_attachments_by_uid(
                &mut session,
                &chunk,
                &mailbox,
                &domain,
                port,
                &username,
                &password,
                &connection_label,
                &shutdown,
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
    shutdown: ShutdownSignal,
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

            if shutdown_requested(&shutdown) {
                info!(
                    "Shutdown requested by user: database writer task finished current record and is exiting"
                );
                break;
            }
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
#[allow(clippy::too_many_arguments)]
fn spawn_fetch_task(
    domain: String,
    port: u16,
    username: String,
    password: String,
    mailbox: String,
    provider_name: String,
    db_sender: tokio::sync::mpsc::Sender<DatabaseOperations>,
    shutdown: ShutdownSignal,
) -> JoinHandle<anyhow::Result<()>> {
    info!("Spawning fetch task for {}", mailbox);

    tokio::spawn(async move {
        let fetch_started_at = Instant::now();

        let client = get_client(&domain, port).await?;
        info!("Connected successfully");

        let mut session = get_session(&username, &password, client).await?;

        let _mailbox = session.examine(&mailbox)?;

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
            domain.clone(),
            port,
            username.clone(),
            password.clone(),
            mailbox.clone(),
            uids,
            shutdown.clone(),
        )
        .await;
        info!("Attachment fetching done for {}", mailbox);

        for message in &messages {
            // Unique ID for every email. There is also sequence number but don't depend on it because it changes.
            let uid = message.uid.unwrap_or(0);

            // Extract fields. This is an edge case: there may be an email for which the
            // envelope would not be retrieved.
            let Some(envelope) = message.envelope() else {
                tracing::warn!("Envelope is None for UID {}, skipping", uid);
                continue;
            };
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
                provider: provider_name.clone(),
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

            if shutdown_requested(&shutdown) {
                info!(
                    "Shutdown requested by user: fetch task for {} finished current record and is exiting",
                    mailbox
                );
                break;
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
pub async fn sync_emails(syncargs: SyncArgs, db_manager: &DatabaseManager) -> anyhow::Result<()> {
    let provider = if let Some(provider) = syncargs.provider {
        provider
    } else {
        db_manager
            .get_default_provider_opt()
            .await?
            .unwrap_or_default()
    };

    if provider.is_empty() {
        anyhow::bail!(
            "No provider specified and no default provider set. Please pass --provider <NAME> or set a default provider using 'emailyzer providers default --name <NAME>'"
        );
    }

    if !db_manager.provider_exists(provider.clone()).await? {
        anyhow::bail!("Provider '{}' not found", provider);
    }

    let provider = db_manager.get_provider_data(provider.clone()).await?;
    let provider_password = get_password(provider.id)?;

    info!("Syncing emails from {}", provider.name);

    // Shared shutdown signal: flips to `true` once Ctrl+C is pressed, so every task below
    // finishes the record it's currently on and exits instead of being killed mid-record.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let shutdown_listener_task = spawn_shutdown_listener(shutdown_tx);

    // Channel to send database operations to database writer task.
    let (db_sender, db_receiver) = channel(DB_CHANNEL_BUFFER_SIZE);
    let db_task = spawn_db_writer_task(db_receiver, shutdown_rx.clone());
    let inbox_fetch_task = spawn_fetch_task(
        provider.url.clone(),
        provider.port,
        provider.username.clone(),
        provider_password.clone(),
        provider.inbox_label.clone(),
        provider.name.clone(),
        db_sender.clone(),
        shutdown_rx.clone(),
    );
    let sent_emails_fetch_task = spawn_fetch_task(
        provider.url.clone(),
        provider.port,
        provider.username.clone(),
        provider_password.clone(),
        provider.sent_label.clone(),
        provider.name.clone(),
        db_sender,
        shutdown_rx,
    );

    // Await both concurrently-running tasks. Prefer surfacing the writer task's error (if any)
    // since it's the true root cause; the fetch task's error in that scenario is just a generic
    // "channel closed" message caused by the writer task exiting first.
    let (inbox_fetch_result, db_result, sent_emails_fetch_result) =
        tokio::join!(inbox_fetch_task, db_task, sent_emails_fetch_task);
    // Intentional defensive cleanup, not legacy leftover: if the sync finished on its own
    // (no Ctrl+C was ever pressed), `shutdown_listener_task` is still parked awaiting a signal
    // that will never come. Aborting it here stops that task from lingering past this function.
    shutdown_listener_task.abort();
    db_result??;
    inbox_fetch_result??;
    sent_emails_fetch_result??;

    Ok(())
}
