use std::path::Path;

use rusqlite::Connection;
use tracing::{debug, info};

use crate::types::Email;

pub async fn create_or_open_db<P: AsRef<Path>>(path: P) -> anyhow::Result<Connection> {
    let conn = Connection::open(path)?;
    info!("Connection to the SQLite database successfully established");

    // Create required tables if required
    if !conn.table_exists(None, "emails")? {
        info!("Couldn't find `emails` table, creating it");
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
        info!("Table `emails` created successfully");
    } else {
        info!("`emails` table is already present.");
    }
    conn.is_autocommit();
    Ok(conn)
}

pub async fn create_email_entry(conn: &Connection, email: Email) -> anyhow::Result<()> {
    let uid = email.uid;

    if uid_exists(conn, uid).await? {
        debug!("Email with UID {} already exists, skipping", uid);
        return Ok(());
    }
    conn.execute(
        "INSERT INTO emails (uid, subject, sender, receiver, has_attachment, timestamp, body) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        (
            uid,
            email.subject,
            email.sender,
            email.receiver,
            email.attachment,
            email.timestamp,
            email.body
        ),
    )?;
    debug!(
        "Email with UID {} is saved successfully in database",
        email.uid
    );

    Ok(())
}

pub async fn get_last_fetched_uid(conn: &Connection) -> anyhow::Result<u32> {
    let last_uid: u32 = conn.query_row("SELECT COALESCE(MAX(uid), 0) FROM emails", [], |row| {
        row.get(0)
    })?;
    Ok(last_uid)
}

pub async fn uid_exists(conn: &Connection, uid: u32) -> anyhow::Result<bool> {
    let email_uid = conn.query_one("SELECT uid from emails where uid = ?", [uid], |row| {
        row.get::<usize, u32>(0)
    });

    let uid_exists = match email_uid {
        Ok(_) => true,
        Err(e) => match e {
            rusqlite::Error::QueryReturnedNoRows => false,
            _ => return Err(e.into()),
        },
    };

    Ok(uid_exists)
}
