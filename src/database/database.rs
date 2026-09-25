use std::str::FromStr;

use crate::email::{Email, ReceiverStats, SenderStats};
use crate::providers::Provider;
use crate::{database::DatabaseLocation, email::types::DEFAULT_PROVIDER_KEY};
use sqlx::{
    Row as _,
    sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
    },
};
use tracing::{debug, info};

pub struct DatabaseManager {
    conn: SqlitePool,
}

impl DatabaseManager {
    /// Creates and returns a new instance of DatabaseManager, connecting according to `location`
    /// and running pending migrations.
    pub async fn new(location: DatabaseLocation<'_>) -> anyhow::Result<Self> {
        let (opts, description) = match location {
            DatabaseLocation::File(path) => (
                SqliteConnectOptions::from_str(format!("sqlite://{path}").as_str())?
                    .create_if_missing(true)
                    // WAL lets writers and readers (e.g. an external SQLite editor) work
                    // concurrently without blocking each other, and only fsyncs at checkpoints
                    // instead of on every single-statement commit like the default rollback
                    // journal does. That per-commit fsync is what was causing the multi-second
                    // "slow statement" warnings on each INSERT.
                    .journal_mode(SqliteJournalMode::Wal)
                    // NORMAL is safe under WAL: at worst a crash loses the last few committed
                    // transactions (not corrupting the database), while avoiding an fsync on
                    // every commit.
                    .synchronous(SqliteSynchronous::Normal)
                    // Required for ON DELETE CASCADE (e.g. deleting a provider also deletes its
                    // emails/stats) to actually take effect: SQLite ignores foreign key actions
                    // unless this pragma is enabled per-connection.
                    .foreign_keys(true),
                format!("SQLite database {path}"),
            ),
            #[cfg(test)]
            DatabaseLocation::Memory => (
                SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true),
                "in-memory SQLite database".to_string(),
            ),
        };

        // Only one connection is ever needed: DatabaseManager is used exclusively from a single
        // writer task. For an in-memory database, capping at 1 is required anyway: each connection would
        // otherwise get its own separate, empty in-memory database.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await?;
        info!("Connection to the {} successfully established", description);

        // Run pending migrations embedded from the "migrations" directory at compile time.
        sqlx::migrate!("./migrations").run(&pool).await?;
        info!("Database migrations applied successfully");

        Ok(DatabaseManager { conn: pool })
    }

    /// Create an entry in "emails" database table.
    ///
    /// Uses `INSERT OR IGNORE` instead of a separate existence check followed by an insert,
    /// folding what used to be two round-trips (SELECT + INSERT) into one.
    pub async fn create_email_entry(&self, email: Email) -> anyhow::Result<()> {
        let uid = email.uid;

        let mut conn = self.conn.acquire().await?;

        let result = sqlx::query(
            r#"
                INSERT OR IGNORE INTO emails (uid, subject, sender, read_status, receiver, has_attachment, timestamp, body, label, provider_id)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, (SELECT id FROM providers WHERE provider_name = ?10))
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
        .bind(email.label)
        .bind(email.provider)
        .execute(&mut *conn).await?;

        if result.rows_affected() == 0 {
            debug!("Email with UID {} already exists, skipping", uid);
        }

        Ok(())
    }

    /// Create an entry in "sender_email_stats" database table
    #[allow(clippy::too_many_arguments)]
    pub async fn create_sender_email_stats_entry(
        &self,
        sender: String,
        total_emails: u32,
        read_emails: u32,
        unread_emails: u32,
        attachment_count: u32,
        no_attachment_count: u32,
        provider: String,
    ) -> anyhow::Result<()> {
        let mut conn = self.conn.acquire().await?;

        sqlx::query(
            r#"
                INSERT INTO sender_email_stats (sender, total_emails, read_emails, unread_emails, attachment_count, no_attachment_count, provider_id)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, (SELECT id FROM providers WHERE provider_name = ?7))
            "#
        )
        .bind(sender)
        .bind(total_emails)
        .bind(read_emails)
        .bind(unread_emails)
        .bind(attachment_count)
        .bind(no_attachment_count)
        .bind(provider)
        .execute(&mut *conn).await?;

        Ok(())
    }

    /// Create an entry in "receiver_email_stats" database table
    #[allow(clippy::too_many_arguments)]
    pub async fn create_receiver_email_stats_entry(
        &self,
        receiver: String,
        total_emails: u32,
        read_emails: u32,
        unread_emails: u32,
        attachment_count: u32,
        no_attachment_count: u32,
        provider: String,
    ) -> anyhow::Result<()> {
        let mut conn = self.conn.acquire().await?;

        sqlx::query(
            r#"
                INSERT INTO receiver_email_stats (receiver, total_emails, read_emails, unread_emails, attachment_count, no_attachment_count, provider_id)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, (SELECT id FROM providers WHERE provider_name = ?7))
            "#
        )
        .bind(receiver)
        .bind(total_emails)
        .bind(read_emails)
        .bind(unread_emails)
        .bind(attachment_count)
        .bind(no_attachment_count)
        .bind(provider)
        .execute(&mut *conn).await?;

        Ok(())
    }

    /// Delete all entries from "sender_email_stats" database table
    pub async fn delete_all_sender_email_stats_entries(
        &self,
        provider: String,
    ) -> anyhow::Result<()> {
        let mut conn = self.conn.acquire().await?;

        sqlx::query(
            "DELETE FROM sender_email_stats WHERE provider_id = (SELECT id FROM providers WHERE provider_name = ?1)",
        )
        .bind(provider)
        .execute(&mut *conn)
        .await?;
        Ok(())
    }

    /// Delete all entries from "receiver_email_stats" database table
    pub async fn delete_all_receiver_email_stats_entries(
        &self,
        provider: String,
    ) -> anyhow::Result<()> {
        let mut conn = self.conn.acquire().await?;

        sqlx::query(
            "DELETE FROM receiver_email_stats WHERE provider_id = (SELECT id FROM providers WHERE provider_name = ?1)",
        )
        .bind(provider)
        .execute(&mut *conn)
        .await?;
        Ok(())
    }

    /// Read all emails from "emails" database table for specific label
    pub async fn read_emails_from_database(
        &self,
        label: &str,
        provider: String,
    ) -> anyhow::Result<Vec<Email>> {
        let mut conn = self.conn.acquire().await?;

        let emails = sqlx::query(
            r#"
                SELECT e.uid, e.subject, e.sender, e.read_status, e.receiver, e.has_attachment, e.timestamp, e.body, e.label
                FROM emails e
                JOIN providers p ON p.id = e.provider_id
                WHERE e.label = ?1 AND p.provider_name = ?2
            "#,
        )
        .bind(label.to_lowercase())
        .bind(provider)
        .try_map(|row: sqlx::sqlite::SqliteRow| {
            Ok(Email {
                uid: row.try_get("uid")?,
                subject: row.try_get("subject")?,
                sender: row.try_get("sender")?,
                read_status: row.try_get("read_status")?,
                receiver: row.try_get("receiver")?,
                attachment: row.try_get::<i16, _>("has_attachment")?,
                timestamp: row.try_get("timestamp")?,
                body: row.try_get("body")?,
                label: row.try_get("label")?,
                provider: "".to_string(), // No need for it
            })
        })
        .fetch_all(&mut *conn)
        .await?;

        Ok(emails)
    }

    /// Read sender email stats from "sender_email_stats" database table
    pub async fn read_sender_email_stats(
        &self,
        provider: String,
    ) -> anyhow::Result<Vec<SenderStats>> {
        let mut conn = self.conn.acquire().await?;

        let senders = sqlx::query(
            r#"
                SELECT s.sender, s.total_emails, s.read_emails, s.unread_emails, s.attachment_count, s.no_attachment_count
                FROM sender_email_stats s
                JOIN providers p ON p.id = s.provider_id
                WHERE p.provider_name = ?1
            "#,
        )
        .bind(provider)
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

    /// Read receiver email stats from "receiver_email_stats" database table
    pub async fn read_receiver_email_stats(
        &self,
        provider: String,
    ) -> anyhow::Result<Vec<ReceiverStats>> {
        let mut conn = self.conn.acquire().await?;

        let receivers = sqlx::query(
            r#"
                SELECT r.receiver, r.total_emails, r.read_emails, r.unread_emails, r.attachment_count, r.no_attachment_count
                FROM receiver_email_stats r
                JOIN providers p ON p.id = r.provider_id
                WHERE p.provider_name = ?1
            "#,
        )
        .bind(provider)
        .try_map(|row: sqlx::sqlite::SqliteRow| {
            Ok(ReceiverStats {
                receiver: row.try_get("receiver")?,
                total_emails: row.try_get("total_emails")?,
                read_emails: row.try_get("read_emails")?,
                unread_emails: row.try_get("unread_emails")?,
                attachment_count: row.try_get("attachment_count")?,
                no_attachment_count: row.try_get("no_attachment_count")?,
            })
        })
        .fetch_all(&mut *conn)
        .await?;

        Ok(receivers)
    }

    /// Create an entry in "providers" database table.
    ///
    /// Returns the newly-assigned `id`, so callers can use it as the key for storing the
    /// provider's password in the system keyring.
    pub async fn create_provider(&self, provider: Provider) -> anyhow::Result<i64> {
        let mut conn = self.conn.acquire().await?;

        let result = sqlx::query(
            r#"
                INSERT INTO providers (provider_name, imap_server_url, imap_server_port, username, inbox_label, sent_label) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(provider.name.clone())
        .bind(provider.url)
        .bind(provider.port)
        .bind(provider.username)
        .bind(provider.inbox_label)
        .bind(provider.sent_label)
        .execute(&mut *conn)
        .await?;

        Ok(result.last_insert_rowid())
    }

    /// Get the `id` of a provider by name, if it exists. Used by callers that need the id as
    /// the key for the provider's password in the system keyring (e.g. before deleting it).
    pub async fn get_provider_id(&self, name: String) -> anyhow::Result<Option<i64>> {
        let mut conn = self.conn.acquire().await?;

        let result = sqlx::query("SELECT id FROM providers WHERE provider_name = ?1")
            .bind(name)
            .fetch_optional(&mut *conn)
            .await?;

        Ok(result.map(|row| row.get::<i64, _>(0)))
    }

    /// Delete an entry from "providers" database table.
    ///
    /// Returns the number of rows affected, so callers can determine whether a provider with
    /// the given name actually existed. Deleting a provider cascades (via the `ON DELETE
    /// CASCADE` foreign key on `provider_id`) to delete all emails and sender/receiver stats
    /// associated with it.
    pub async fn delete_provider(&self, name: String) -> anyhow::Result<u64> {
        let mut conn = self.conn.acquire().await?;

        let result = sqlx::query("DELETE FROM providers WHERE provider_name = ?1")
            .bind(name)
            .execute(&mut *conn)
            .await?;

        Ok(result.rows_affected())
    }

    /// Count how many emails are associated with the given provider name, so callers can warn
    /// the user before a cascading delete removes them.
    pub async fn count_emails_for_provider(&self, name: String) -> anyhow::Result<i64> {
        let mut conn = self.conn.acquire().await?;

        let result = sqlx::query(
            r#"
                SELECT COUNT(*) FROM emails e
                JOIN providers p ON p.id = e.provider_id
                WHERE p.provider_name = ?1
            "#,
        )
        .bind(name)
        .fetch_one(&mut *conn)
        .await?;

        Ok(result.get::<i64, _>(0))
    }

    /// Read all providers from "providers" database table
    pub async fn read_providers(&self) -> anyhow::Result<Vec<Provider>> {
        let mut conn = self.conn.acquire().await?;

        let providers = sqlx::query(
            r#"SELECT id, provider_name, imap_server_url, imap_server_port, username, inbox_label, sent_label FROM providers"#,
        )
        .try_map(|row: sqlx::sqlite::SqliteRow| {
            let provider = Provider {
                id: row.try_get("id")?,
                name: row.try_get("provider_name")?,
                url: row.try_get("imap_server_url")?,
                port: row.try_get("imap_server_port")?,
                username: row.try_get("username")?,
                inbox_label: row.try_get("inbox_label")?,
                sent_label: row.try_get("sent_label")?,
            };
            Ok(provider)
        })
        .fetch_all(&mut *conn)
        .await?;

        Ok(providers)
    }

    /// Set default provider in "settings" database table
    pub async fn set_default_provider(&self, name: String) -> anyhow::Result<()> {
        let mut conn = self.conn.acquire().await?;

        sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)")
            .bind(DEFAULT_PROVIDER_KEY)
            .bind(name)
            .execute(&mut *conn)
            .await?;

        Ok(())
    }

    /// Get default provider from "settings" database table, if one is set
    pub async fn get_default_provider_opt(&self) -> anyhow::Result<Option<String>> {
        let mut conn = self.conn.acquire().await?;

        let result = sqlx::query("SELECT value FROM settings WHERE key = ?1")
            .bind(DEFAULT_PROVIDER_KEY)
            .fetch_optional(&mut *conn)
            .await?;

        Ok(result.map(|row| row.get::<String, _>(0)))
    }

    /// Check if a provider with the given name exists in the "providers" database table
    pub async fn provider_exists(&self, name: String) -> anyhow::Result<bool> {
        let mut conn = self.conn.acquire().await?;

        let result = sqlx::query("SELECT COUNT(*) FROM providers WHERE provider_name = ?1")
            .bind(name)
            .fetch_one(&mut *conn)
            .await?;

        Ok(result.get::<i64, _>(0) > 0)
    }

    /// Get provider data
    pub async fn get_provider_data(&self, provider: String) -> anyhow::Result<Provider> {
        let mut conn = self.conn.acquire().await?;

        let provider: Provider =
            sqlx::query(r#"SELECT id, imap_server_url, imap_server_port, username, inbox_label, sent_label FROM providers where provider_name = ?1"#)
                .bind(provider.clone())
                .try_map(|row: sqlx::sqlite::SqliteRow| {
                    let provider = Provider {
                            id: row.try_get("id")?,
                            name: provider.clone(),
                            url: row.try_get("imap_server_url")?,
                            port: row.try_get("imap_server_port")?,
                            username: row.try_get("username")?,
                            inbox_label: row.try_get("inbox_label")?,
                            sent_label: row.try_get("sent_label")?,
                        };
                        Ok(provider)
                })
                .fetch_one(&mut *conn)
                .await?;

        Ok(provider)
    }
}
