use std::io::Read;

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
