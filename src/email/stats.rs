use std::collections::HashMap;

use comfy_table::{ContentArrangement, Table, presets::UTF8_FULL};
use tracing::info;

use crate::{
    ReceiverSortBy, ReceiversArgs, SenderSortBy, SendersArgs,
    database::DatabaseManager,
    email::types::{Email, ReceiverStats, SenderStats},
};

pub async fn get_sender_stats(
    sendersargs: SendersArgs,
    db_manager: &DatabaseManager,
) -> anyhow::Result<()> {
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

    let provider = if let Some(provider) = sendersargs.provider {
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

    if sendersargs.refresh {
        info!("User has passed --refresh flag. Reading emails from database");
        let provider_data = db_manager.get_provider_data(provider.clone()).await?;
        let emails: Vec<Email> = db_manager
            .read_emails_from_database(&provider_data.inbox_label, provider.clone())
            .await?;
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
        db_manager
            .delete_all_sender_email_stats_entries(provider.clone())
            .await?;

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
                    provider.clone(),
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
        let senders_stats = db_manager.read_sender_email_stats(provider).await?;

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

pub async fn get_receiver_stats(
    receiversargs: ReceiversArgs,
    db_manager: &DatabaseManager,
) -> anyhow::Result<()> {
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

    let provider = if let Some(provider) = receiversargs.provider {
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

    if receiversargs.refresh {
        info!("User has passed --refresh flag. Reading emails from database");
        let provider_data = db_manager.get_provider_data(provider.clone()).await?;
        let emails: Vec<Email> = db_manager
            .read_emails_from_database(&provider_data.sent_label.to_lowercase(), provider.clone())
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
        db_manager
            .delete_all_receiver_email_stats_entries(provider.clone())
            .await?;

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
                    provider.clone(),
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
        let receivers_stats = db_manager
            .read_receiver_email_stats(provider.clone())
            .await?;

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
