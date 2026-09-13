use std::path::Path;

use rusqlite::Connection;
use tracing::{debug, info};

use crate::types::Email;

/// 1. Creates or open a database at given path.
/// 2. Create required tables if they don't exist.
pub async fn create_or_open_db<P: AsRef<Path>>(path: P) -> anyhow::Result<Connection> {
    let conn = Connection::open(path)?;
    info!("Connection to the SQLite database successfully established");

    // Create required tables if required
    if !conn.table_exists(None, "emails")? {
        info!("Couldn't find `emails` table, creating it");
        conn.execute(
            "CREATE TABLE emails 
        (
            uid INTEGER PRIMARY KEY,
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

    if !conn.table_exists(None, "sender_email_stats")? {
        info!("Couldn't find `sender_email_stats` table, creating it");
        conn.execute(
            "CREATE TABLE sender_email_stats
        (
            sender TEXT PRIMARY KEY,
            total_emails INTEGER
        )",
            (),
        )?;
        info!("Table `sender_email_stats` created successfully");
    } else {
        info!("`sender_email_stats` table is already present.");
    }

    conn.is_autocommit();
    Ok(conn)
}

/// Create an entry in "emails" database table
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

/// Create an entry in "sender_email_stats" database table
pub async fn create_sender_email_stats_entry(
    conn: &Connection,
    sender: String,
    total_emails: u32,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO sender_email_stats (sender, total_emails) VALUES (?1, ?2)",
        (sender, total_emails),
    )?;
    Ok(())
}

/// Delete all entries from "sender_email_stats" database table
pub async fn delete_all_sender_email_stats_entries(conn: &Connection) -> anyhow::Result<()> {
    conn.execute("DELETE FROM sender_email_stats", [])?;
    Ok(())
}

/// Get the last fetched UID from "emails" database table
pub async fn get_last_fetched_uid(conn: &Connection) -> anyhow::Result<u32> {
    let last_uid: u32 = conn.query_row("SELECT COALESCE(MAX(uid), 0) FROM emails", [], |row| {
        row.get(0)
    })?;
    Ok(last_uid)
}

/// Check if email with given UID already exists in "emails" database table
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

/// Read all emails from "emails" database table
pub async fn read_emails(conn: &Connection) -> anyhow::Result<Vec<Email>> {
    let mut stmt = conn.prepare("SELECT uid, sender, timestamp FROM emails")?;
    let emails: Vec<Email> = stmt
        .query_map([], |row| {
            Ok(Email {
                uid: row.get(0)?,
                subject: "".to_string(),
                sender: row.get(1)?,
                receiver: "".to_string(),
                attachment: false,
                timestamp: row.get(2)?,
                body: "".to_string(),
            })
        })?
        .map(|row| match row {
            Ok(email) => email,
            Err(e) => {
                tracing::error!("Error while reading email: {}", e);
                Email {
                    uid: 0,
                    subject: "".to_string(),
                    sender: "".to_string(),
                    receiver: "".to_string(),
                    attachment: false,
                    timestamp: "".to_string(),
                    body: "".to_string(),
                }
            }
        })
        .collect();

    Ok(emails)
}

/// Read sender email stats from "sender_email_stats" database table
pub async fn read_sender_email_stats(conn: &Connection) -> anyhow::Result<Vec<(String, u32)>> {
    let mut stmt = conn.prepare("SELECT sender, total_emails FROM sender_email_statsss")?;
    let senders: Vec<(String, u32)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .map(|row| match row {
            Ok(sender) => sender,
            Err(e) => {
                tracing::error!("Error while reading sender email stats: {}", e);
                ("".to_string(), 0)
            }
        })
        .collect();

    Ok(senders)
}
