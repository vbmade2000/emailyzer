use crate::{
    cli::{Cli, Commands, ProviderSubcommand},
    database::{DATABASE_URL, DatabaseLocation, DatabaseManager},
    email::{get_provider_mailboxes, get_receiver_stats, get_sender_stats, sync_emails},
    grpc::grpc_server::launch,
    providers::{add_provider, delete_provider, list_providers, set_default_provider},
    util::set_tracing,
};
use clap::Parser;

mod cli;
mod database;
mod email;
mod grpc;
mod password_store;
mod providers;
mod util;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    set_tracing(cli.log_file)?;

    let db_manager = DatabaseManager::new(DatabaseLocation::File(DATABASE_URL)).await?;

    let commands = cli.command;
    match commands {
        Commands::Sync(syncargs) => {
            sync_emails(syncargs, &db_manager).await?;
        }
        Commands::Senders(sendersargs) => {
            get_sender_stats(sendersargs, &db_manager).await?;
        }
        Commands::Receivers(receiversargs) => {
            get_receiver_stats(receiversargs, &db_manager).await?;
        }
        Commands::Providers(providerargs) => match providerargs.subcommand {
            ProviderSubcommand::Add(add_provider_args) => {
                add_provider(add_provider_args, &db_manager).await?;
            }
            ProviderSubcommand::Delete(delete_provider_args) => {
                delete_provider(delete_provider_args, &db_manager).await?;
            }
            ProviderSubcommand::List => {
                list_providers(&db_manager).await?;
            }
            ProviderSubcommand::Default(default_provider_args) => {
                set_default_provider(default_provider_args, &db_manager).await?;
            }
            ProviderSubcommand::Mailboxes(mailboxes_args) => {
                get_provider_mailboxes(mailboxes_args).await?;
            }
        },
        Commands::Serve => {
            launch().await?;
        }
    }

    Ok(())
}
