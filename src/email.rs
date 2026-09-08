use std::net::TcpStream;

use imap::{Client, Session, types::Fetch};
use native_tls::{TlsConnector, TlsStream};
use tokio_imap::types::{Address, Envelope};

/// Retrieves email credentials from env vars.
/// You can set it in current shell or .bashrc as below.
/// export GMAIL_USERNAME="your-gmail-username"
/// export GMAIL_PWD="your-gmail-password"
pub fn get_credentials() -> anyhow::Result<(String, String)> {
    println!("Retrieving credentials from env var");
    // IMP: Do NOT hardcode your Gmail password or App Password in source code.
    Ok((
        std::env::var("GMAIL_USERNAME")?,
        std::env::var("GMAIL_PWD")?,
    ))
}

/// Create instance of TlsConnector to validate Gmail's TLS certificate
pub fn get_tls_connector() -> anyhow::Result<TlsConnector> {
    println!("Building TLS Connector to validate Gmail certificate");
    Ok(TlsConnector::builder().build()?)
}

/// Create a client to connect to Gmail
/// 1. Create instance of TlsConnector
/// 2. Create an imap client
pub fn get_client(domain: &str, port: u16) -> anyhow::Result<Client<TlsStream<TcpStream>>> {
    println!("Creating a client");
    let tls = get_tls_connector()?;
    Ok(imap::connect((domain, port), domain, &tls)?)
}

/// Create a session instance
pub fn get_session(
    username: &str,
    password: &str,
    client: Client<TlsStream<TcpStream>>,
) -> anyhow::Result<Session<TlsStream<TcpStream>>> {
    println!("Creating a session for interaction");
    let session = client.login(username, password).map_err(|error| error.0)?;
    println!("Authentication successful!");
    Ok(session)
}

/// Extract subject field from envelope
pub fn get_subject(envelope: &Envelope) -> String {
    envelope
        .subject
        .and_then(|subject| std::str::from_utf8(subject).ok())
        .unwrap_or("<no subject>")
        .to_string()
}

// Extract sender/from field from envelope
pub fn get_sender(envelope: &Envelope) -> String {
    envelope
        .from
        .as_ref()
        .and_then(|addresses| addresses.first())
        .map(format_address)
        .unwrap_or_else(|| "<unknown sender>".to_string())
}

/// Extract receiver from envelope
pub fn get_receiver(envelope: &Envelope) -> String {
    envelope
        .to
        .as_ref()
        .and_then(|addresses| addresses.first())
        .map(format_address)
        .unwrap_or_else(|| "<unknown receiver>".to_string())
}

/// Extract datetime from envelope
pub fn get_datetime(envelope: &Envelope) -> String {
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
