use crate::email::Email;

/// Database operations
pub enum DatabaseOperations {
    CreateEmailEntry(Email),
}

/// Where a `DatabaseManager` should open its SQLite connection.
#[allow(dead_code)]
pub enum DatabaseLocation<'a> {
    /// A database file on disk at the given path.
    File(&'a str),
    /// An in-memory database, useful for unit tests: it's isolated per-instance, requires no
    /// filesystem access/cleanup, and disappears once the `DatabaseManager` (and its pool) is
    /// dropped.
    #[cfg(test)]
    Memory,
}

// Contants
pub const DATABASE_URL: &str = "emailyzer.db";
