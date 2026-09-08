use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use tracing::{info, level_filters::LevelFilter};
use tracing_subscriber::EnvFilter;

use crate::email::fetch_emails;

mod database;
mod email;
mod types;

#[derive(Parser, Debug)]
#[command(
    name = "emailyzer",
    version = "0.1.0",
    author = "Malhar Vora <vbmade2000 at gmail dot com>",
    about = "A fast email analysis tool"
)]
struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Fetch emails from the provider
    Fetch(FetchArgs),
}

#[derive(Args, Debug)]
pub struct FetchArgs {
    /// Path to the log file
    #[arg(short, long)]
    pub log_file: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let commands = cli.command;
    match commands {
        Commands::Fetch(_fetchargs) => {
            set_tracing().await?;
            fetch_emails().await?;

            // TODO: Handle log file arg
            // if let Some(log_file) = fetchargs.log_file {
            //     println!("Found log file: {:?}", log_file);
            // }
        }
    }

    Ok(())
}

async fn set_tracing() -> anyhow::Result<()> {
    /*
        This helps in setting log level from RUST_LOG env var. eg: export RUST_LOG=debug. If not set,
        it will default to INFO.
    */
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    let subscriber = tracing_subscriber::fmt()
        .with_file(true)
        .with_line_number(true)
        .with_thread_ids(false)
        .with_target(true)
        .with_env_filter(env_filter)
        .finish();

    tracing::subscriber::set_global_default(subscriber)?;
    info!("Tracing subscriber set successfully");
    Ok(())
}
