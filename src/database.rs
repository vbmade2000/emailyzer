use std::str::FromStr;

use crate::types::{Email, SenderStats};
use sqlx::{
    Row as _,
    sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions},
};
use tracing::{debug, info};

pub struct DatabaseManager {
    conn: SqlitePool,
}

impl DatabaseManager {
    /// Creates and returns a new instance of DatabaseManager.
    pub async fn new<P: AsRef<str>>(path: P) -> anyhow::Result<Self> {
        let opts = SqliteConnectOptions::from_str(format!("sqlite://{}", path.as_ref()).as_str())?
            .create_if_missing(true);

        // Only one connection is ever needed: DatabaseManager is used exclusively from a single
        // writer task. Capping the pool at 1 avoids multiple physical SQLite connections racing
        // on the WAL file, which can otherwise trigger "disk I/O error" (SQLITE_IOERR_SHORT_READ).
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await?;
        info!(
            "Connection to the SQLite database {} successfully established",
            path.as_ref()
        );

        // Run pending migrations embedded from the "migrations" directory at compile time.
        sqlx::migrate!("./migrations").run(&pool).await?;
        info!("Database migrations applied successfully");

        Ok(DatabaseManager { conn: pool })
    }

    /// Create an entry in "emails" database table
    pub async fn create_email_entry(&self, email: Email) -> anyhow::Result<()> {
        let uid = email.uid;

        if self.uid_exists(uid).await? {
            debug!("Email with UID {} already exists, skipping", uid);
            return Ok(());
        }

        let mut conn = self.conn.acquire().await?;

        sqlx::query(
            r#"
                INSERT INTO emails (uid, subject, sender, read_status, receiver, has_attachment, timestamp, body) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#
        )
        .bind(uid)
        .bind(email.subject)
        .bind(email.sender)
        .bind(email.read_status)
        .bind(email.receiver)
        .bind(email.attachment)
        .bind(email.timestamp)
        .bind(email.body)
        .execute(&mut *conn).await?;

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
        let mut conn = self.conn.acquire().await?;

        sqlx::query(
            r#"
                INSERT INTO sender_email_stats (sender, total_emails, read_emails, unread_emails, attachment_count, no_attachment_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#
        )
        .bind(sender)
        .bind(total_emails)
        .bind(read_emails)
        .bind(unread_emails)
        .bind(attachment_count)
        .bind(no_attachment_count)
        .execute(&mut *conn).await?;

        Ok(())
    }

    /// Delete all entries from "sender_email_stats" database table
    pub async fn delete_all_sender_email_stats_entries(&self) -> anyhow::Result<()> {
        let mut conn = self.conn.acquire().await?;

        sqlx::query("DELETE FROM sender_email_stats")
            .execute(&mut *conn)
            .await?;
        Ok(())
    }

    /// Get the last fetched UID from "emails" database table
    pub async fn _get_last_fetched_uid(&self) -> anyhow::Result<u32> {
        let mut conn = self.conn.acquire().await?;

        let last_uid: u32 = sqlx::query_scalar("SELECT COALESCE(MAX(uid), 0) FROM emails")
            .fetch_one(&mut *conn)
            .await?;

        Ok(last_uid)
    }

    /// Check if email with given UID already exists in "emails" database table
    pub async fn uid_exists(&self, uid: u32) -> anyhow::Result<bool> {
        let mut conn = self.conn.acquire().await?;

        let email_uid: Option<u32> = sqlx::query_scalar("SELECT uid FROM emails WHERE uid = ?")
            .bind(uid)
            .fetch_optional(&mut *conn)
            .await?;

        Ok(email_uid.is_some())
    }

    /// Read all emails from "emails" database table
    pub async fn read_emails_from_database(&self) -> anyhow::Result<Vec<Email>> {
        let mut conn = self.conn.acquire().await?;

        let emails = sqlx::query(
            r#"
                SELECT uid, subject, sender, read_status, receiver, has_attachment, timestamp, body
                FROM emails
            "#,
        )
        .try_map(|row: sqlx::sqlite::SqliteRow| {
            Ok(Email {
                uid: row.try_get("uid")?,
                subject: row.try_get("subject")?,
                sender: row.try_get("sender")?,
                read_status: row.try_get("read_status")?,
                receiver: row.try_get("receiver")?,
                attachment: row.try_get("has_attachment")?,
                timestamp: row.try_get("timestamp")?,
                body: row.try_get("body")?,
            })
        })
        .fetch_all(&mut *conn)
        .await?;

        Ok(emails)
    }

    /// Read sender email stats from "sender_email_stats" database table
    pub async fn read_sender_email_stats(&self) -> anyhow::Result<Vec<SenderStats>> {
        let mut conn = self.conn.acquire().await?;

        let senders = sqlx::query(
            r#"
                SELECT sender, total_emails, read_emails, unread_emails, attachment_count, no_attachment_count
                FROM sender_email_stats
            "#,
        )
        .try_map(|row: sqlx::sqlite::SqliteRow| {
            Ok(SenderStats {
                sender: row.try_get("sender")?,
                total_emails: row.try_get("total_emails")?,
                read_emails: row.try_get("read_emails")?,
                unread_emails: row.try_get("unread_emails")?,
                attachment_count: row.try_get("attachment_count")?,
                no_attachment_count: row.try_get("no_attachment_count")?,
            })
        })
        .fetch_all(&mut *conn)
        .await?;

        Ok(senders)
    }
}
