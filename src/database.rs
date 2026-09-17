use std::path::Path;

use rusqlite::Connection;
use tracing::{debug, info};

use crate::types::Email;

pub struct DatabaseManager {
    conn: Connection,
}

impl DatabaseManager {
    /// Creates and returns a new instance of DatabaseManager.
    /// 1. Creates or open a database at given path.
    /// 2. Create required tables if they don't exist.
    pub async fn new<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let conn = Connection::open(&path)?;
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
            read_status bool,
            has_attachment bool NOT NULL,
            timestamp TEXT NOT NULL,
            body TEXT,
            label TEXT DEFAULT 'INBOX'
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
            total_emails INTEGER,
            read_emails INTEGER,
            unread_emails INTEGER,
            attachment_count INTEGER,
            no_attachment_count INTEGER
        )",
                (),
            )?;
            info!("Table `sender_email_stats` created successfully");
        } else {
            info!("`sender_email_stats` table is already present.");
        }

        conn.is_autocommit();

        Ok(DatabaseManager { conn })
    }

    /// Create an entry in "emails" database table
    pub async fn create_email_entry(&self, email: Email) -> anyhow::Result<()> {
        let uid = email.uid;

        if self.uid_exists(uid).await? {
            debug!("Email with UID {} already exists, skipping", uid);
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO emails (uid, subject, sender, read_status, receiver, has_attachment, timestamp, body) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                uid,
                email.subject,
                email.sender,
                email.read_status,
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
        &self,
        sender: String,
        total_emails: u32,
        read_emails: u32,
        unread_emails: u32,
        attachment_count: u32,
        no_attachment_count: u32,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO sender_email_stats (sender, total_emails, read_emails, unread_emails, attachment_count, no_attachment_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (sender, total_emails, read_emails, unread_emails, attachment_count, no_attachment_count),
        )?;
        Ok(())
    }

    /// Delete all entries from "sender_email_stats" database table
    pub async fn delete_all_sender_email_stats_entries(&self) -> anyhow::Result<()> {
        self.conn.execute("DELETE FROM sender_email_stats", [])?;
        Ok(())
    }

    /// Get the last fetched UID from "emails" database table
    pub async fn _get_last_fetched_uid(&self) -> anyhow::Result<u32> {
        let last_uid: u32 =
            self.conn
                .query_row("SELECT COALESCE(MAX(uid), 0) FROM emails", [], |row| {
                    row.get(0)
                })?;
        Ok(last_uid)
    }

    /// Check if email with given UID already exists in "emails" database table
    pub async fn uid_exists(&self, uid: u32) -> anyhow::Result<bool> {
        let email_uid = self
            .conn
            .query_one("SELECT uid from emails where uid = ?", [uid], |row| {
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
    pub async fn read_emails_from_database(&self) -> anyhow::Result<Vec<Email>> {
        let mut stmt = self
            .conn
            .prepare("SELECT uid, sender, timestamp, read_status, has_attachment FROM emails")?;
        let emails: Vec<Email> = stmt
            .query_map([], |row| {
                Ok(Email {
                    uid: row.get(0)?,
                    subject: "".to_string(),
                    sender: row.get(1)?,
                    read_status: row.get(3)?,
                    receiver: "".to_string(),
                    attachment: row.get(4)?,
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
                        read_status: false,
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
    pub async fn read_sender_email_stats(
        &self,
    ) -> anyhow::Result<Vec<(String, u32, u32, u32, u32, u32)>> {
        let mut stmt = self.conn.prepare(
            "SELECT sender, total_emails, read_emails, unread_emails, attachment_count, no_attachment_count FROM sender_email_stats",
        )?;
        let senders: Vec<(String, u32, u32, u32, u32, u32)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })?
            .map(|row| match row {
                Ok(r) => (r.0, r.1, r.2, r.3, r.4, r.5),
                Err(e) => {
                    tracing::error!("Error while reading sender email stats: {}", e);
                    ("".to_string(), 0, 0, 0, 0, 0)
                }
            })
            .collect();

        Ok(senders)
    }
}
