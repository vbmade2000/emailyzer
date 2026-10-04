use std::{
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};

use comfy_table::{ContentArrangement, Table, presets::UTF8_FULL};
use imap::{Client, Session};
use native_tls::{TlsConnector, TlsStream};
use tracing::info;

use crate::{MailboxesArgs, util::read_secret};

/// Create instance of TlsConnector to validate Gmail's TLS certificate
pub(crate) async fn get_tls_connector() -> anyhow::Result<TlsConnector> {
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
pub(crate) async fn get_client(
    domain: &str,
    port: u16,
) -> anyhow::Result<Client<TlsStream<TcpStream>>> {
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
pub(crate) async fn get_session(
    username: &str,
    password: &str,
    client: Client<TlsStream<TcpStream>>,
) -> anyhow::Result<Session<TlsStream<TcpStream>>> {
    info!("Creating a session for interaction");
    let session = client.login(username, password).map_err(|error| error.0)?;
    info!("Authentication successful. Session established successfully");
    Ok(session)
}

/// Opens a fresh IMAP connection/session and `EXAMINE`s `mailbox`. Used to recover after a
/// session's underlying byte stream may have been left desynced (see `fetch_attachments_by_uid`),
/// since reusing a desynced session for further commands causes the `imap` crate to panic (via an
/// internal `assert_eq!` on response tags) rather than return a `Result`.
pub(crate) async fn reconnect_session(
    domain: &str,
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

/// Returns a list of labels from the email account. Ex. INBOX, Drafts, Sent mails etc
async fn get_mailboxes(session: &mut Session<TlsStream<TcpStream>>) -> anyhow::Result<Vec<String>> {
    let mailboxes = session.list(None, Some("*"))?;

    let mailbox_names = mailboxes.iter().map(|m| m.name().to_string()).collect();

    Ok(mailbox_names)
}

/// Connects directly to the IMAP server described by `args` and prints out all the
/// mailboxes/labels available on the account, so the user can pick the correct values for
/// `--inbox-label`/`--sent-label` when adding a provider.
pub async fn get_provider_mailboxes(args: MailboxesArgs) -> anyhow::Result<()> {
    info!("Fetching mailboxes from {}", args.url);

    let password = read_secret(&args.password_file)?;

    let client = get_client(&args.url, args.port).await?;
    let mut session = get_session(&args.username, &password, client).await?;

    let mailboxes = get_mailboxes(&mut session).await?;

    session.logout()?;

    let mut table = Table::new();
    table.set_header(vec!["Mailbox/Label"]);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.load_style(UTF8_FULL.with_rounded_corners());

    for mailbox in mailboxes {
        table.add_row(vec![mailbox]);
    }

    println!("{table}");

    Ok(())
}
