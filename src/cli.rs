use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "emailyzer",
    version = "0.1.0",
    author = "Malhar Vora <vbmade2000 at gmail dot com>",
    about = "Yet another email analysis tool"
)]
pub struct Cli {
    /// Path to the log file
    #[arg(
        short,
        long,
        value_name = "FILE",
        global = true,
        default_value = "emailyzer.log"
    )]
    pub log_file: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Fetch emails from the provider
    Sync(SyncArgs),
    // Analyze the fetched emails for senders
    Senders(SendersArgs),
    /// Analyze the fetched emails for senders
    Receivers(ReceiversArgs),
    /// Manage email providers
    Providers(ProvidersArgs),
    /// Launch server
    Serve,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, clap::ValueEnum)]
pub enum SenderSortBy {
    Emails,
    Sender,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, clap::ValueEnum)]
pub enum ReceiverSortBy {
    Emails,
    Receiver,
}

#[derive(Args, Debug)]
pub struct SendersArgs {
    /// Perform fresh calculations from existing database entries
    #[arg(short, long)]
    pub refresh: bool,
    /// Filter by senders. eg: --sender "sender1@example.com" --sender "sender2@example.com"
    #[arg(short, long, value_name = "SENDER", action = clap::ArgAction::Append)]
    pub sender: Vec<String>,
    /// Sort by total emails received
    #[arg(short = 'o', long, value_name = "SORT_BY", value_parser = clap::value_parser!(SenderSortBy))]
    pub sort_by: Option<SenderSortBy>,
    /// Show top N senders by no of emails
    #[arg(short, long, value_name = "N", value_parser = clap::value_parser!(u8))]
    pub top: Option<u8>,
    /// Provider name
    #[arg(short, long, value_name = "PROVIDER")]
    pub provider: Option<String>,
}

#[derive(Args, Debug)]
pub struct ProvidersArgs {
    #[command(subcommand)]
    pub subcommand: ProviderSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum ProviderSubcommand {
    /// Add a new provider
    Add(AddProviderArgs),
    /// Delete a provider
    Delete(DeleteProviderArgs),
    /// List all configured providers
    List,
    /// Set default provider
    Default(DefaultProviderArgs),
    /// Fetch and display all mailboxes/labels available for a provider
    Mailboxes(MailboxesArgs),
}

#[derive(Args, Clone, Debug)]
pub struct MailboxesArgs {
    /// IMAP server URL
    #[arg(short, long, value_name = "URL")]
    pub url: String,
    /// IMAP server port
    #[arg(short, long, value_name = "PORT")]
    pub port: u16,
    /// Username
    #[arg(short = 'e', long, value_name = "USERNAME")]
    pub username: String,
    /// File containing the password, or '-' to read it from stdin. Avoid passing passwords
    /// directly as flag values, since they can leak into shell history and process listings.
    #[arg(short = 'w', long, value_name = "FILE")]
    pub password_file: String,
}

#[derive(Args, Clone, Debug)]
pub struct AddProviderArgs {
    /// Name of the provider
    #[arg(short, long, value_name = "NAME")]
    pub name: String,
    /// IMAP server URL
    #[arg(short, long, value_name = "URL")]
    pub url: String,
    /// IMAP server port
    #[arg(short, long, value_name = "PORT")]
    pub port: u16,
    /// Username
    #[arg(short = 'e', long, value_name = "USERNAME")]
    pub username: String,
    /// File containing the password, or '-' to read it from stdin. Avoid passing passwords
    /// directly as flag values, since they can leak into shell history and process listings.
    #[arg(short = 'w', long, value_name = "FILE")]
    pub password_file: String,
    /// Mailbox/label name for the inbox, e.g. "INBOX" for Gmail.
    #[arg(short = 'i', long, value_name = "INBOX_LABEL")]
    pub inbox_label: String,
    /// Mailbox/label name for sent emails, e.g. "[Gmail]/Sent Mail" for Gmail.
    #[arg(short = 's', long, value_name = "SENT_LABEL")]
    pub sent_label: String,
}

#[derive(Args, Clone, Debug)]
pub struct DeleteProviderArgs {
    /// Name of the provider
    #[arg(short, long, value_name = "NAME")]
    pub name: String,
    /// Skip the confirmation prompt
    #[arg(short, long)]
    pub force: bool,
}

#[derive(Args, Clone, Debug)]
pub struct DefaultProviderArgs {
    /// Name of the provider
    #[arg(short, long, value_name = "NAME")]
    pub name: String,
}

#[derive(Args, Debug)]
pub struct ReceiversArgs {
    /// Perform fresh calculations from existing database entries
    #[arg(short, long)]
    pub refresh: bool,
    /// Filter by receivers. eg: --receiver "receiver1@example.com" --receiver "receiver2@example.com"
    #[arg(short = 'e', long, value_name = "RECEIVER", action = clap::ArgAction::Append)]
    pub receiver: Vec<String>,
    /// Sort by total emails sent
    #[arg(short = 'o', long, value_name = "SORT_BY", value_parser = clap::value_parser!(ReceiverSortBy))]
    pub sort_by: Option<ReceiverSortBy>,
    /// Show top N receivers by no of emails
    #[arg(short, long, value_name = "N", value_parser = clap::value_parser!(u8))]
    pub top: Option<u8>,
    /// Provider name
    #[arg(short, long, value_name = "PROVIDER")]
    pub provider: Option<String>,
}

#[derive(Args, Debug)]
pub struct SyncArgs {
    /// Provider name
    #[arg(short, long, value_name = "PROVIDER")]
    pub provider: Option<String>,
}
