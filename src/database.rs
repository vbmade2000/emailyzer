use std::path::Path;

use rusqlite::Connection;

pub fn create_or_open_db<P: AsRef<Path>>(path: P) -> anyhow::Result<Connection> {
    let conn = Connection::open(path)?;

    // Create required tables
    conn.execute(
        "CREATE TABLE emails 
        (
            uid INTEGER PRIMAEY KEY,
            subject TEXT,
            sender TEXT NOT NULL,
            receiver TEXT NOT NULL,
            has_attachment bool NOT NULL,
            timestamp TEXT NOT NULL,
            body TEXT
        )",
        (),
    )?;

    conn.is_autocommit();
    Ok(conn)
}
