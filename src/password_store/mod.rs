use keyring::Entry;

const SERVICE_NAME: &str = "emailyzer";

/// Keyring entries are keyed by the provider's immutable database `id` rather than its
/// (mutable, user-supplied) name, so a future provider rename can't orphan the stored
/// password.
fn entry_for(provider_id: i64) -> anyhow::Result<Entry> {
    Ok(Entry::new(SERVICE_NAME, &provider_id.to_string())?)
}

/// Store password in keyring
pub fn store_password(provider_id: i64, password: &str) -> anyhow::Result<()> {
    let entry = entry_for(provider_id)?;
    entry.set_password(password)?;
    Ok(())
}

/// Get password from keyring
pub fn get_password(provider_id: i64) -> anyhow::Result<String> {
    let entry = entry_for(provider_id)?;
    entry.get_password().map_err(|e| match e {
        keyring::Error::NoEntry => anyhow::anyhow!(
            "No stored password found for this provider. Try deleting and re-adding it."
        ),
        e => e.into(),
    })
}

/// Delete password from keyring.
///
/// It's not an error if no password was stored for this provider (e.g. it was already removed
/// or never stored due to a partial failure) — the caller's job (removing the provider) is
/// still considered done.
pub fn delete_password(provider_id: i64) -> anyhow::Result<()> {
    let entry = entry_for(provider_id)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}
