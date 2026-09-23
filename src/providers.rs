use comfy_table::{ContentArrangement, Table, presets::UTF8_FULL};
use tracing::info;

use crate::{
    AddProviderArgs, DefaultProviderArgs, DeleteProviderArgs,
    database::{DatabaseLocation, DatabaseManager},
    types::{DATABASE_URL, Provider},
};

/// Add a new provider
pub async fn add_provider(args: AddProviderArgs) -> anyhow::Result<()> {
    if args.name.is_empty()
        || args.url.is_empty()
        || args.username.is_empty()
        || args.password.is_empty()
    {
        anyhow::bail!("Name, URL, username and password are required");
    }

    let provider = Provider {
        name: args.name.clone(),
        url: args.url,
        port: args.port,
        username: args.username,
        password: args.password,
        inbox_label: args.inbox_label,
        sent_label: args.sent_label,
    };

    let db_manager = DatabaseManager::new(DatabaseLocation::File(DATABASE_URL)).await?;
    db_manager.create_provider(provider).await?;

    info!("Provider {} added successfully", &args.name);

    Ok(())
}

/// Delete a provider
pub async fn delete_provider(args: DeleteProviderArgs) -> anyhow::Result<()> {
    if args.name.is_empty() {
        anyhow::bail!("Name is required");
    }

    let db_manager = DatabaseManager::new(DatabaseLocation::File(DATABASE_URL)).await?;
    let rows_affected = db_manager.delete_provider(args.name.clone()).await?;

    if rows_affected == 0 {
        anyhow::bail!("Provider '{}' not found", &args.name);
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
        anyhow::bail!("Provider '{}' not found", &provider_name);
    }

    db_manager
        .set_default_provider(provider_name.clone())
        .await?;

    info!("Default provider set to {}", &provider_name);

    Ok(())
}
