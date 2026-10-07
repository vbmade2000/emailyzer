use std::{io::Read, path::PathBuf};

use std::fs::File;
use std::sync::Mutex;

use tracing::{info, level_filters::LevelFilter};
use tracing_subscriber::{EnvFilter, fmt::writer::BoxMakeWriter};

/// Reports whether stdin is an interactive terminal.
///
/// In tests this can be overridden via `set_stdin_interactive_override`, since a test's
/// stdin state depends on how the test binary happens to be invoked (e.g. a real TTY when
/// run directly in a terminal vs. redirected/piped under CI or an IDE test runner) rather
/// than on the behavior being tested.
#[cfg(not(test))]
pub fn is_stdin_interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

#[cfg(test)]
thread_local! {
    static STDIN_INTERACTIVE_OVERRIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub fn is_stdin_interactive() -> bool {
    STDIN_INTERACTIVE_OVERRIDE
        .with(|o| o.get())
        .unwrap_or_else(|| {
            use std::io::IsTerminal;
            std::io::stdin().is_terminal()
        })
}

/// Overrides the value returned by `is_stdin_interactive()` for the current thread, so tests
/// don't depend on whether the test binary's stdin happens to be a real TTY. `None` restores
/// the real terminal check. Since `cargo test` runs each test on its own thread, this doesn't
/// need any cross-test synchronization.
#[cfg(test)]
pub fn set_stdin_interactive_override(value: Option<bool>) {
    STDIN_INTERACTIVE_OVERRIDE.with(|o| o.set(value));
}

/// Reads a secret (e.g. a password) either from a file, or from stdin if `path` is `-`.
///
/// This avoids passing secrets directly as command-line flag values, which would leak them
/// into shell history and process listings (e.g. `ps`).
///
/// Trailing newlines are trimmed since secret files are commonly created with a trailing
/// newline by text editors or `echo`.
pub fn read_secret(path: &str) -> anyhow::Result<String> {
    let mut contents = String::new();

    if path == "-" {
        std::io::stdin()
            .read_to_string(&mut contents)
            .map_err(|e| anyhow::anyhow!("Failed to read secret from stdin: {}", e))?;
    } else {
        contents = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read secret from file '{}': {}", path, e))?;
    }

    let secret = contents.trim_end_matches(['\n', '\r']).to_string();

    if secret.is_empty() {
        anyhow::bail!("Secret read from '{}' is empty", path);
    }

    Ok(secret)
}

pub fn set_tracing(log_file: Option<PathBuf>) -> anyhow::Result<()> {
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
