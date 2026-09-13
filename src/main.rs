use std::fs::File;
use std::path::PathBuf;
use std::sync::Mutex;

use clap::{Args, Parser, Subcommand};
use tracing::{info, level_filters::LevelFilter};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::writer::BoxMakeWriter;

use crate::email::fetch_emails;

mod database;
mod email;
mod types;

#[derive(Parser, Debug)]
#[command(
    name = "emailyzer",
    version = "0.1.0",
    author = "Malhar Vora <vbmade2000 at gmail dot com>",
    about = "Yet another email analysis tool"
)]
struct Cli {
    /// Path to the log file
    #[arg(short, long, value_name = "FILE", global = true)]
    pub log_file: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Fetch emails from the provider
    Fetch,
    // Analyze the fetched emails
    Analyze(AnalyzeArgs),
}

#[derive(Args, Debug)]
pub struct AnalyzeArgs {
    /// Perform fresh calculations from existing database entries
    #[arg(short, long)]
    pub refresh: Option<bool>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    set_tracing(cli.log_file)?;

    let commands = cli.command;
    match commands {
        Commands::Fetch => {
            fetch_emails().await?;
        }
        Commands::Analyze(_analyzeargs) => {
            // analyze_emails(analyzeargs.refresh.unwrap_or(false)).await?;
            todo!()
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
