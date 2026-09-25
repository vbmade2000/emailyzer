use comfy_table::{ContentArrangement, Table, presets::UTF8_FULL};
use tracing::{debug, info};

use super::types::Provider;
use crate::{
    AddProviderArgs, DefaultProviderArgs, DeleteProviderArgs,
    database::{DATABASE_URL, DatabaseLocation, DatabaseManager},
    password_store::{delete_password, store_password},
    util::read_secret,
};

/// Add a new provider
pub async fn add_provider(args: AddProviderArgs) -> anyhow::Result<()> {
    if args.name.is_empty()
        || args.url.is_empty()
        || args.username.is_empty()
        || args.password_file.is_empty()
    {
        anyhow::bail!("Name, URL, username and password file are required");
    }

    let password = read_secret(&args.password_file)?;

    let provider = Provider {
        id: 0, // Ignored on insert; the database assigns the real id.
        name: args.name.clone(),
        url: args.url,
        port: args.port,
        username: args.username,
        inbox_label: args.inbox_label,
        sent_label: args.sent_label,
    };

    let db_manager = DatabaseManager::new(DatabaseLocation::File(DATABASE_URL)).await?;
    let provider_id = db_manager.create_provider(provider).await?;
    debug!("Provider {} added to database", args.name);

    // Store password in keyring, keyed by the provider's stable id (not its mutable name).
    // Roll back the database insert if this fails, so we don't leave a provider with no
    // stored password behind.
    if let Err(e) = store_password(provider_id, &password) {
        if let Err(rollback_err) = db_manager.delete_provider(args.name.clone()).await {
            tracing::warn!(
                "Failed to store password for provider '{}', and failed to roll back its database entry: {}. \
                 The provider was left in the database with no stored password; delete it manually with `providers delete --force`.",
                args.name,
                rollback_err
            );
        }
        return Err(e);
    }

    info!("Provider {} added successfully", &args.name);

    Ok(())
}

/// Delete a provider
pub async fn delete_provider(args: DeleteProviderArgs) -> anyhow::Result<()> {
    if args.name.is_empty() {
        anyhow::bail!("Name is required");
    }

    let db_manager = DatabaseManager::new(DatabaseLocation::File(DATABASE_URL)).await?;
    let email_count = db_manager
        .count_emails_for_provider(args.name.clone())
        .await?;

    if !args.force {
        use std::io::IsTerminal;

        let warning = if email_count > 0 {
            format!(
                "Warning: this will also permanently delete {} email(s) associated with provider '{}'. ",
                email_count, args.name
            )
        } else {
            String::new()
        };

        if !std::io::stdin().is_terminal() {
            anyhow::bail!(
                "{}Refusing to delete provider '{}' without confirmation. Pass --force to skip the prompt.",
                warning,
                args.name
            );
        }

        let confirmed = inquire::Confirm::new(&format!(
            "{}Are you sure you want to delete provider '{}'?",
            warning, args.name
        ))
        .with_default(false)
        .prompt()?;

        if !confirmed {
            println!("Aborted");
            return Ok(());
        }
    }

    // Fetch the id before deleting the row, since it's needed to look up the keyring entry.
    let provider_id = db_manager.get_provider_id(args.name.clone()).await?;

    let rows_affected = db_manager.delete_provider(args.name.clone()).await?;

    if rows_affected == 0 {
        anyhow::bail!("Provider '{}' not found", args.name);
    }

    // Delete password from keyring. The database row is already gone at this point, so treat
    // a failure here as non-fatal (log only) rather than leaving the user with a provider that
    // looks half-deleted.
    if let Some(provider_id) = provider_id
        && let Err(e) = delete_password(provider_id)
    {
        tracing::warn!(
            "Provider '{}' deleted, but failed to remove its password from the keyring: {}",
            args.name,
            e
        );
    }

    info!("Provider {} deleted successfully", &args.name);

    Ok(())
}

/// List all configured providers
pub async fn list_providers() -> anyhow::Result<()> {
    let db_manager = DatabaseManager::new(DatabaseLocation::File(DATABASE_URL)).await?;
    let providers = db_manager.read_providers().await?;

    if providers.is_empty() {
        println!("No providers found");
        return Ok(());
    }

    let default_provider = db_manager.get_default_provider_opt().await?;

    // Print providers using Table
    let mut table = Table::new();
    table.set_header(vec![
        "Name",
        "URL",
        "Port",
        "Username",
        "Inbox Label",
        "Sent Label",
        "Default",
    ]);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.load_style(UTF8_FULL.with_rounded_corners());

    for provider in providers {
        let is_default = default_provider.as_deref() == Some(provider.name.as_str());
        table.add_row(vec![
            provider.name,
            provider.url,
            provider.port.to_string(),
            provider.username,
            provider.inbox_label,
            provider.sent_label,
            if is_default { "Yes" } else { "" }.to_string(),
        ]);
    }

    println!("{table}");

    Ok(())
}

/// Set default provider
pub async fn set_default_provider(args: DefaultProviderArgs) -> anyhow::Result<()> {
    let provider_name = args.name;

    if provider_name.is_empty() {
        anyhow::bail!("Name is required");
    }

    let db_manager = DatabaseManager::new(DatabaseLocation::File(DATABASE_URL)).await?;
    let provider_exists = db_manager.provider_exists(provider_name.clone()).await?;

    if !provider_exists {
        anyhow::bail!("Provider '{}' not found", provider_name);
    }

    db_manager
        .set_default_provider(provider_name.clone())
        .await?;

    info!("Default provider set to {}", &provider_name);

    Ok(())
}
