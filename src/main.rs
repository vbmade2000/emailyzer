use std::fs::File;
use std::path::PathBuf;
use std::sync::Mutex;

use clap::Parser;
use tracing::{info, level_filters::LevelFilter};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::writer::BoxMakeWriter;

use crate::{
    cli::{Cli, Commands, ProviderSubcommand},
    database::{DATABASE_URL, DatabaseLocation, DatabaseManager},
    email::{get_provider_mailboxes, get_receiver_stats, get_sender_stats, sync_emails},
    grpc::grpc_server::launch,
    providers::{add_provider, delete_provider, list_providers, set_default_provider},
};

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

fn set_tracing(log_file: Option<PathBuf>) -> anyhow::Result<()> {
    /*
        This helps in setting log level from RUST_LOG env var. eg: export RUST_LOG=debug. If not set,
        it will default to INFO.
    */
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    let writer = match log_file {
        Some(path) => {
            let file = File::create(path)?;
            // Mutex is required to make sure that multiple threads don't write to the same file at once.
            BoxMakeWriter::new(Mutex::new(file))
        }
        // If log file is not specified, write to stdout. Stdout handles locking internally so no need for Mutex.
        None => BoxMakeWriter::new(std::io::stdout),
    };

    let subscriber = tracing_subscriber::fmt()
        .with_file(true)
        .with_line_number(true)
        .with_thread_ids(false)
        .with_target(true)
        .with_env_filter(env_filter)
        .with_writer(writer)
        .finish();

    tracing::subscriber::set_global_default(subscriber)?;
    info!("Tracing subscriber set successfully");
    Ok(())
}
