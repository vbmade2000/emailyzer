use std::collections::HashMap;

use comfy_table::{ContentArrangement, Table, presets::UTF8_FULL};
use tracing::info;

use crate::{
    cli::{ReceiverSortBy, ReceiversArgs, SenderSortBy, SendersArgs},
    database::DatabaseManager,
    email::types::{Email, ReceiverStats, SenderStats},
    providers::Provider,
};

/// How to order rows before an optional `--top N` truncation. Both `senders`/`receivers`
/// commands expose the same two choices (by email count, or alphabetically by the row's key),
/// just under differently-named CLI enums (`SenderSortBy`/`ReceiverSortBy`).
enum SortField {
    Emails,
    Key,
}

impl From<SenderSortBy> for SortField {
    fn from(value: SenderSortBy) -> Self {
        match value {
            SenderSortBy::Emails => SortField::Emails,
            SenderSortBy::Sender => SortField::Key,
        }
    }
}

impl From<ReceiverSortBy> for SortField {
    fn from(value: ReceiverSortBy) -> Self {
        match value {
            ReceiverSortBy::Emails => SortField::Emails,
            ReceiverSortBy::Receiver => SortField::Key,
        }
    }
}

/// Common behavior needed to aggregate, persist, and display one row of sender/receiver stats.
/// Implemented by both `SenderStats` and `ReceiverStats` so `get_stats` can be written once and
/// shared by `get_sender_stats`/`get_receiver_stats` instead of duplicating the aggregation,
/// sorting, and table-rendering logic for each.
trait StatRow: Default {
    /// Table column / CLI concept name for this row's key, e.g. "Sender" or "Receiver".
    const HEADER: &'static str;

    /// The mailbox/label stats for this row type are aggregated from, e.g. the inbox for senders
    /// and the (lowercased) sent label for receivers.
    fn mailbox_label(provider_data: &Provider) -> String;

    fn key_from_email(email: &Email) -> String;
    fn key(&self) -> &str;
    fn set_key(&mut self, key: String);
    fn total_emails(&self) -> u32;
    fn record_email(&mut self, email: &Email);
    fn to_table_row(&self) -> Vec<String>;

    async fn read_existing(
        db_manager: &DatabaseManager,
        provider: String,
    ) -> anyhow::Result<Vec<Self>>
    where
        Self: Sized;
    async fn delete_all(db_manager: &DatabaseManager, provider: String) -> anyhow::Result<()>;
    async fn persist(&self, db_manager: &DatabaseManager, provider: String) -> anyhow::Result<()>;
}

impl StatRow for SenderStats {
    const HEADER: &'static str = "Sender";

    fn mailbox_label(provider_data: &Provider) -> String {
        provider_data.inbox_label.clone()
    }

    fn key_from_email(email: &Email) -> String {
        email.sender.clone()
    }

    fn key(&self) -> &str {
        &self.sender
    }

    fn set_key(&mut self, key: String) {
        self.sender = key;
    }

    fn total_emails(&self) -> u32 {
        self.total_emails
    }

    fn record_email(&mut self, email: &Email) {
        self.total_emails += 1;
        if email.read_status {
            self.read_emails += 1;
        } else {
            self.unread_emails += 1;
        }
        if email.attachment == 1 {
            self.attachment_count += 1;
        } else {
            self.no_attachment_count += 1;
        }
    }

    fn to_table_row(&self) -> Vec<String> {
        vec![
            self.sender.clone(),
            self.total_emails.to_string(),
            self.read_emails.to_string(),
            self.unread_emails.to_string(),
            self.attachment_count.to_string(),
            self.no_attachment_count.to_string(),
        ]
    }

    async fn read_existing(
        db_manager: &DatabaseManager,
        provider: String,
    ) -> anyhow::Result<Vec<Self>> {
        db_manager.read_sender_email_stats(provider).await
    }

    async fn delete_all(db_manager: &DatabaseManager, provider: String) -> anyhow::Result<()> {
        db_manager
            .delete_all_sender_email_stats_entries(provider)
            .await
    }

    async fn persist(&self, db_manager: &DatabaseManager, provider: String) -> anyhow::Result<()> {
        db_manager
            .create_sender_email_stats_entry(
                self.sender.clone(),
                self.total_emails,
                self.read_emails,
                self.unread_emails,
                self.attachment_count,
                self.no_attachment_count,
                provider,
            )
            .await
    }
}

impl StatRow for ReceiverStats {
    const HEADER: &'static str = "Receiver";

    fn mailbox_label(provider_data: &Provider) -> String {
        provider_data.sent_label.to_lowercase()
    }

    fn key_from_email(email: &Email) -> String {
        email.receiver.clone()
    }

    fn key(&self) -> &str {
        &self.receiver
    }

    fn set_key(&mut self, key: String) {
        self.receiver = key;
    }

    fn total_emails(&self) -> u32 {
        self.total_emails
    }

    fn record_email(&mut self, email: &Email) {
        self.total_emails += 1;
        if email.read_status {
            self.read_emails += 1;
        } else {
            self.unread_emails += 1;
        }
        if email.attachment == 1 {
            self.attachment_count += 1;
        } else {
            self.no_attachment_count += 1;
        }
    }

    fn to_table_row(&self) -> Vec<String> {
        vec![
            self.receiver.clone(),
            self.total_emails.to_string(),
            self.read_emails.to_string(),
            self.unread_emails.to_string(),
            self.attachment_count.to_string(),
            self.no_attachment_count.to_string(),
        ]
    }

    async fn read_existing(
        db_manager: &DatabaseManager,
        provider: String,
    ) -> anyhow::Result<Vec<Self>> {
        db_manager.read_receiver_email_stats(provider).await
    }

    async fn delete_all(db_manager: &DatabaseManager, provider: String) -> anyhow::Result<()> {
        db_manager
            .delete_all_receiver_email_stats_entries(provider)
            .await
    }

    async fn persist(&self, db_manager: &DatabaseManager, provider: String) -> anyhow::Result<()> {
        db_manager
            .create_receiver_email_stats_entry(
                self.receiver.clone(),
                self.total_emails,
                self.read_emails,
                self.unread_emails,
                self.attachment_count,
                self.no_attachment_count,
                provider,
            )
            .await
    }
}

/// Resolves/validates the provider to use, aggregates or reloads `S` stats (depending on
/// `refresh`), and sorts/truncates them. Does not print anything, so it can be reused by both
/// the CLI (`get_stats`, which prints a table) and other consumers like a gRPC handler that need
/// the raw rows instead.
async fn compute_stats<S: StatRow>(
    provider_arg: Option<String>,
    refresh: bool,
    preferred_keys: Vec<String>,
    sort_by: Option<SortField>,
    top: Option<u8>,
    db_manager: &DatabaseManager,
) -> anyhow::Result<Vec<S>> {
    let provider = if let Some(provider) = provider_arg {
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

    info!("Using provider: {}", provider);

    let mut rows: Vec<S> = Vec::new();

    if refresh {
        info!("User has passed --refresh flag. Reading emails from database");
        let provider_data = db_manager.get_provider_data(provider.clone()).await?;
        let emails: Vec<Email> = db_manager
            .read_emails_from_database(&S::mailbox_label(&provider_data), provider.clone())
            .await?;
        info!("Fetched {} emails from database", emails.len());

        let mut stats_by_key: HashMap<String, S> = HashMap::new();

        // Count emails by each unique key (sender/receiver)
        for email in emails {
            let key = S::key_from_email(&email);
            let stat = stats_by_key.entry(key.clone()).or_default();
            stat.set_key(key);
            stat.record_email(&email);
        }

        // Clear database table first to make fresh entries
        S::delete_all(db_manager, provider.clone()).await?;

        // Save the stats in database because user has used --refresh flag. Also, print records on stdout
        for (key, stat) in stats_by_key {
            stat.persist(db_manager, provider.clone()).await?;

            // We show only records from preferred keys if user has passed the corresponding filter flag
            if !preferred_keys.is_empty() && !preferred_keys.contains(&key) {
                continue;
            }

            rows.push(stat);
        }
    } else {
        info!(
            "User has skipped --refresh flag. Reading existing {} email stats from database",
            S::HEADER.to_lowercase()
        );
        let existing_stats = S::read_existing(db_manager, provider).await?;

        if existing_stats.is_empty() {
            info!(
                "No {} email stats found in database. Please use --refresh flag to sync emails first",
                S::HEADER.to_lowercase()
            );
            return Ok(Vec::new());
        }

        for stat in existing_stats {
            // We show only records from preferred keys if user has passed the corresponding filter flag
            if !preferred_keys.is_empty() && !preferred_keys.contains(&stat.key().to_string()) {
                continue;
            }
            rows.push(stat);
        }
    }

    // Sort the rows based on --sort-by flag, if provided. If --sort-by is absent but --top is
    // present, default to sorting by emails so "top N" has a well-defined meaning (highest
    // email counts first).
    match sort_by {
        Some(SortField::Emails) => rows.sort_by_key(|b| std::cmp::Reverse(b.total_emails())),
        Some(SortField::Key) => rows.sort_by(|a, b| a.key().cmp(b.key())),
        None => {
            if top.is_some() {
                rows.sort_by_key(|b| std::cmp::Reverse(b.total_emails()));
            }
        }
    }

    // Show only the top N rows (in whatever order was established above) if --top was passed.
    if let Some(top) = top {
        rows.truncate(top as usize);
    }

    Ok(rows)
}

/// Computes `S` stats (see `compute_stats`) and prints them as a table. Used by the CLI
/// (`get_sender_stats`/`get_receiver_stats`).
async fn show_stats<S: StatRow>(
    provider_arg: Option<String>,
    refresh: bool,
    preferred_keys: Vec<String>,
    sort_by: Option<SortField>,
    top: Option<u8>,
    db_manager: &DatabaseManager,
) -> anyhow::Result<()> {
    let rows = compute_stats::<S>(
        provider_arg,
        refresh,
        preferred_keys,
        sort_by,
        top,
        db_manager,
    )
    .await?;

    // Prepare table for display
    let mut table = Table::new();
    table.set_header(vec![
        S::HEADER,
        "Emails",
        "Read",
        "Unread",
        "Attachment",
        "No Attachment",
    ]);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.load_style(UTF8_FULL.with_rounded_corners());

    for row in &rows {
        table.add_row(row.to_table_row());
    }

    println!("{table}");

    Ok(())
}

pub async fn get_sender_stats(
    sendersargs: SendersArgs,
    db_manager: &DatabaseManager,
) -> anyhow::Result<()> {
    show_stats::<SenderStats>(
        sendersargs.provider,
        sendersargs.refresh,
        sendersargs.sender,
        sendersargs.sort_by.map(SortField::from),
        sendersargs.top,
        db_manager,
    )
    .await
}

pub async fn get_receiver_stats(
    receiversargs: ReceiversArgs,
    db_manager: &DatabaseManager,
) -> anyhow::Result<()> {
    show_stats::<ReceiverStats>(
        receiversargs.provider,
        receiversargs.refresh,
        receiversargs.receiver,
        receiversargs.sort_by.map(SortField::from),
        receiversargs.top,
        db_manager,
    )
    .await
}

/// Computes sender stats and returns them as raw rows (no table/printing), intended for use by
/// the gRPC `get_sender_stats` handler.
pub async fn get_sender_stats_rows(
    sendersargs: SendersArgs,
    db_manager: &DatabaseManager,
) -> anyhow::Result<Vec<SenderStats>> {
    compute_stats::<SenderStats>(
        sendersargs.provider,
        sendersargs.refresh,
        sendersargs.sender,
        sendersargs.sort_by.map(SortField::from),
        sendersargs.top,
        db_manager,
    )
    .await
}

/// Computes receiver stats and returns them as raw rows (no table/printing), intended for use by
/// the gRPC `get_receiver_stats` handler.
pub async fn get_receiver_stats_rows(
    receiversargs: ReceiversArgs,
    db_manager: &DatabaseManager,
) -> anyhow::Result<Vec<ReceiverStats>> {
    compute_stats::<ReceiverStats>(
        receiversargs.provider,
        receiversargs.refresh,
        receiversargs.receiver,
        receiversargs.sort_by.map(SortField::from),
        receiversargs.top,
        db_manager,
    )
    .await
}
