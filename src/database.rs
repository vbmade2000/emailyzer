use std::path::Path;

use rusqlite::Connection;

use crate::types::Email;

pub fn create_or_open_db<P: AsRef<Path>>(path: P) -> anyhow::Result<Connection> {
    let conn = Connection::open(path)?;

    // Create required tables if required
    if !conn.table_exists(None, "emails")? {
        println!("Couldn't find `emails` table, creating it");
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
    } else {
        println!("`emails` table is already present.");
    }
    conn.is_autocommit();
    Ok(conn)
}

pub fn create_email_entry(conn: &Connection, email: Email) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO emails (uid, subject, sender, receiver, has_attachment, timestamp, body) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        (
            email.uid,
            email.subject,
            email.sender,
            email.receiver,
            email.attachment,
            email.timestamp,
            email.body
        ),
    )?;

    Ok(())
}

pub fn get_last_fetched_uid(conn: &Connection) -> anyhow::Result<u32> {
    let last_uid: u32 = conn.query_row("SELECT COALESCE(MAX(uid), 0) FROM emails", [], |row| {
        row.get(0)
    })?;
    Ok(last_uid)
}
