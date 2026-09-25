#[cfg(test)]
mod database_tests {
    use crate::database::{DatabaseLocation, DatabaseManager};
    use crate::email::Email;
    use crate::providers::Provider;

    /// Fresh isolated database per test: each in-memory SQLite instance is private to
    /// its single-connection pool and disappears when the manager is dropped, so no
    /// locking or cleanup is needed.
    async fn test_db() -> DatabaseManager {
        DatabaseManager::new(DatabaseLocation::Memory)
            .await
            .expect("in-memory test database")
    }

    fn make_provider(name: &str) -> Provider {
        Provider {
            id: 0,
            name: name.to_string(),
            url: "imap.example.com".to_string(),
            port: 993,
            username: "user@example.com".to_string(),
            inbox_label: "inbox".to_string(),
            sent_label: "sent".to_string(),
        }
    }

    fn make_email(uid: u32, label: &str, provider: &str) -> Email {
        Email {
            uid,
            subject: format!("Subject {uid}"),
            sender: "alice@example.com".to_string(),
            read_status: false,
            receiver: "bob@example.com".to_string(),
            attachment: 0,
            timestamp: "Mon, 1 Jan 2024 00:00:00 +0000".to_string(),
            body: "Hello".to_string(),
            label: label.to_string(),
            provider: provider.to_string(),
        }
    }

    async fn seed_provider(db: &DatabaseManager, name: &str) -> i64 {
        db.create_provider(make_provider(name))
            .await
            .expect("seed provider")
    }

    // providers ---------------------------------------------------------------

    #[tokio::test]
    async fn create_provider_returns_id_and_read_providers_finds_it() {
        let db = test_db().await;
        let id = seed_provider(&db, "gmail").await;
        assert!(id > 0);

        let providers = db.read_providers().await.unwrap();
        assert_eq!(providers.len(), 1);
        let provider = &providers[0];
        assert_eq!(provider.id, id);
        assert_eq!(provider.name, "gmail");
        assert_eq!(provider.url, "imap.example.com");
        assert_eq!(provider.port, 993);
        assert_eq!(provider.username, "user@example.com");
        assert_eq!(provider.inbox_label, "inbox");
        assert_eq!(provider.sent_label, "sent");
    }

    #[tokio::test]
    async fn create_provider_rejects_duplicate_name() {
        let db = test_db().await;
        seed_provider(&db, "gmail").await;
        assert!(
            db.create_provider(make_provider("gmail")).await.is_err(),
            "provider_name is UNIQUE"
        );
    }

    #[tokio::test]
    async fn get_provider_id_returns_id_or_none() {
        let db = test_db().await;
        let id = seed_provider(&db, "gmail").await;
        assert_eq!(
            db.get_provider_id("gmail".to_string()).await.unwrap(),
            Some(id)
        );
        assert_eq!(
            db.get_provider_id("missing".to_string()).await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn provider_exists_reports_presence() {
        let db = test_db().await;
        assert!(!db.provider_exists("gmail".to_string()).await.unwrap());
        seed_provider(&db, "gmail").await;
        assert!(db.provider_exists("gmail".to_string()).await.unwrap());
    }

    #[tokio::test]
    async fn get_provider_data_returns_provider() {
        let db = test_db().await;
        let id = seed_provider(&db, "gmail").await;
        let provider = db.get_provider_data("gmail".to_string()).await.unwrap();
        assert_eq!(provider.id, id);
        assert_eq!(provider.name, "gmail");
        assert_eq!(provider.url, "imap.example.com");
        assert_eq!(provider.port, 993);
        assert_eq!(provider.username, "user@example.com");
    }

    #[tokio::test]
    async fn get_provider_data_fails_for_missing_provider() {
        let db = test_db().await;
        assert!(
            db.get_provider_data("missing".to_string()).await.is_err(),
            "fetch_one finds no row"
        );
    }

    #[tokio::test]
    async fn delete_provider_returns_affected_rows() {
        let db = test_db().await;
        seed_provider(&db, "gmail").await;
        assert_eq!(db.delete_provider("gmail".to_string()).await.unwrap(), 1);
        assert!(!db.provider_exists("gmail".to_string()).await.unwrap());
        // Deleting again touches nothing.
        assert_eq!(db.delete_provider("gmail".to_string()).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn delete_provider_cascades_to_emails_and_stats() {
        let db = test_db().await;
        seed_provider(&db, "gmail").await;
        db.create_email_entry(make_email(1, "inbox", "gmail"))
            .await
            .unwrap();
        db.create_sender_email_stats_entry(
            "alice@example.com".to_string(),
            1,
            0,
            1,
            0,
            1,
            "gmail".to_string(),
        )
        .await
        .unwrap();
        db.create_receiver_email_stats_entry(
            "bob@example.com".to_string(),
            1,
            0,
            1,
            0,
            1,
            "gmail".to_string(),
        )
        .await
        .unwrap();

        assert_eq!(db.delete_provider("gmail".to_string()).await.unwrap(), 1);
        assert_eq!(
            db.count_emails_for_provider("gmail".to_string())
                .await
                .unwrap(),
            0
        );
        assert!(
            db.read_sender_email_stats("gmail".to_string())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            db.read_receiver_email_stats("gmail".to_string())
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn count_emails_for_provider_scopes_to_provider() {
        let db = test_db().await;
        seed_provider(&db, "gmail").await;
        seed_provider(&db, "outlook").await;
        db.create_email_entry(make_email(1, "inbox", "gmail"))
            .await
            .unwrap();
        db.create_email_entry(make_email(2, "inbox", "gmail"))
            .await
            .unwrap();
        // Note: the primary key is (uid, label) across all providers, so the second
        // provider needs a distinct uid here.
        db.create_email_entry(make_email(3, "inbox", "outlook"))
            .await
            .unwrap();

        assert_eq!(
            db.count_emails_for_provider("gmail".to_string())
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            db.count_emails_for_provider("outlook".to_string())
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.count_emails_for_provider("missing".to_string())
                .await
                .unwrap(),
            0
        );
    }

    // emails ------------------------------------------------------------------

    #[tokio::test]
    async fn create_and_read_email_entry_roundtrip() {
        let db = test_db().await;
        seed_provider(&db, "gmail").await;
        let mut email = make_email(42, "inbox", "gmail");
        email.read_status = true;
        email.attachment = 1;
        db.create_email_entry(email).await.unwrap();

        let emails = db
            .read_emails_from_database("inbox", "gmail".to_string())
            .await
            .unwrap();
        assert_eq!(emails.len(), 1);
        let read = &emails[0];
        assert_eq!(read.uid, 42);
        assert_eq!(read.subject, "Subject 42");
        assert_eq!(read.sender, "alice@example.com");
        assert!(read.read_status);
        assert_eq!(read.receiver, "bob@example.com");
        assert_eq!(read.attachment, 1);
        assert_eq!(read.timestamp, "Mon, 1 Jan 2024 00:00:00 +0000");
        assert_eq!(read.body, "Hello");
        assert_eq!(read.label, "inbox");
    }

    #[tokio::test]
    async fn create_email_entry_ignores_duplicate_uid_and_label() {
        let db = test_db().await;
        seed_provider(&db, "gmail").await;
        db.create_email_entry(make_email(1, "inbox", "gmail"))
            .await
            .unwrap();
        // INSERT OR IGNORE: no error, no second row.
        db.create_email_entry(make_email(1, "inbox", "gmail"))
            .await
            .unwrap();

        let emails = db
            .read_emails_from_database("inbox", "gmail".to_string())
            .await
            .unwrap();
        assert_eq!(emails.len(), 1);
    }

    #[tokio::test]
    async fn create_email_entry_allows_same_uid_under_different_labels() {
        let db = test_db().await;
        seed_provider(&db, "gmail").await;
        // The primary key is (uid, label): UIDs repeat across mailboxes.
        db.create_email_entry(make_email(1, "inbox", "gmail"))
            .await
            .unwrap();
        db.create_email_entry(make_email(1, "sent", "gmail"))
            .await
            .unwrap();

        assert_eq!(
            db.read_emails_from_database("inbox", "gmail".to_string())
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            db.read_emails_from_database("sent", "gmail".to_string())
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn create_email_entry_ignores_unknown_provider() {
        let db = test_db().await;
        // The provider_id subselect yields NULL for an unknown provider, violating NOT
        // NULL — but INSERT OR IGNORE turns that into a silent no-op rather than an
        // error, so the email is skipped.
        db.create_email_entry(make_email(1, "inbox", "missing"))
            .await
            .unwrap();
        assert!(
            db.read_emails_from_database("inbox", "missing".to_string())
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            db.count_emails_for_provider("missing".to_string())
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn read_emails_filters_by_label_and_provider() {
        let db = test_db().await;
        seed_provider(&db, "gmail").await;
        seed_provider(&db, "outlook").await;
        db.create_email_entry(make_email(1, "inbox", "gmail"))
            .await
            .unwrap();
        db.create_email_entry(make_email(2, "sent", "gmail"))
            .await
            .unwrap();
        // Distinct uid: the primary key is (uid, label) across all providers.
        db.create_email_entry(make_email(3, "inbox", "outlook"))
            .await
            .unwrap();

        let emails = db
            .read_emails_from_database("inbox", "gmail".to_string())
            .await
            .unwrap();
        assert_eq!(emails.len(), 1);
        assert_eq!(emails[0].uid, 1);

        // The lookup lowercases the label, matching how sync stores it.
        let emails = db
            .read_emails_from_database("INBOX", "gmail".to_string())
            .await
            .unwrap();
        assert_eq!(emails.len(), 1);

        let emails = db
            .read_emails_from_database("drafts", "gmail".to_string())
            .await
            .unwrap();
        assert!(emails.is_empty());
    }

    // sender / receiver stats ---------------------------------------------------

    #[tokio::test]
    async fn create_and_read_sender_stats_roundtrip() {
        let db = test_db().await;
        seed_provider(&db, "gmail").await;
        db.create_sender_email_stats_entry(
            "alice@example.com".to_string(),
            10,
            7,
            3,
            4,
            6,
            "gmail".to_string(),
        )
        .await
        .unwrap();

        let stats = db
            .read_sender_email_stats("gmail".to_string())
            .await
            .unwrap();
        assert_eq!(stats.len(), 1);
        let entry = &stats[0];
        assert_eq!(entry.sender, "alice@example.com");
        assert_eq!(entry.total_emails, 10);
        assert_eq!(entry.read_emails, 7);
        assert_eq!(entry.unread_emails, 3);
        assert_eq!(entry.attachment_count, 4);
        assert_eq!(entry.no_attachment_count, 6);
    }

    #[tokio::test]
    async fn create_and_read_receiver_stats_roundtrip() {
        let db = test_db().await;
        seed_provider(&db, "gmail").await;
        db.create_receiver_email_stats_entry(
            "bob@example.com".to_string(),
            5,
            2,
            3,
            1,
            4,
            "gmail".to_string(),
        )
        .await
        .unwrap();

        let stats = db
            .read_receiver_email_stats("gmail".to_string())
            .await
            .unwrap();
        assert_eq!(stats.len(), 1);
        let entry = &stats[0];
        assert_eq!(entry.receiver, "bob@example.com");
        assert_eq!(entry.total_emails, 5);
        assert_eq!(entry.read_emails, 2);
        assert_eq!(entry.unread_emails, 3);
        assert_eq!(entry.attachment_count, 1);
        assert_eq!(entry.no_attachment_count, 4);
    }

    #[tokio::test]
    async fn delete_all_sender_stats_scopes_to_provider() {
        let db = test_db().await;
        seed_provider(&db, "gmail").await;
        seed_provider(&db, "outlook").await;
        db.create_sender_email_stats_entry(
            "alice@example.com".to_string(),
            1,
            1,
            0,
            0,
            1,
            "gmail".to_string(),
        )
        .await
        .unwrap();
        db.create_sender_email_stats_entry(
            "carol@example.com".to_string(),
            2,
            2,
            0,
            1,
            1,
            "outlook".to_string(),
        )
        .await
        .unwrap();

        db.delete_all_sender_email_stats_entries("gmail".to_string())
            .await
            .unwrap();
        assert!(
            db.read_sender_email_stats("gmail".to_string())
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            db.read_sender_email_stats("outlook".to_string())
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn delete_all_receiver_stats_scopes_to_provider() {
        let db = test_db().await;
        seed_provider(&db, "gmail").await;
        seed_provider(&db, "outlook").await;
        db.create_receiver_email_stats_entry(
            "bob@example.com".to_string(),
            1,
            1,
            0,
            0,
            1,
            "gmail".to_string(),
        )
        .await
        .unwrap();
        db.create_receiver_email_stats_entry(
            "dave@example.com".to_string(),
            2,
            2,
            0,
            1,
            1,
            "outlook".to_string(),
        )
        .await
        .unwrap();

        db.delete_all_receiver_email_stats_entries("gmail".to_string())
            .await
            .unwrap();
        assert!(
            db.read_receiver_email_stats("gmail".to_string())
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            db.read_receiver_email_stats("outlook".to_string())
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn delete_all_stats_on_empty_tables_is_ok() {
        let db = test_db().await;
        seed_provider(&db, "gmail").await;
        db.delete_all_sender_email_stats_entries("gmail".to_string())
            .await
            .unwrap();
        db.delete_all_receiver_email_stats_entries("gmail".to_string())
            .await
            .unwrap();
    }

    // settings ------------------------------------------------------------------

    #[tokio::test]
    async fn default_provider_roundtrip() {
        let db = test_db().await;
        assert_eq!(db.get_default_provider_opt().await.unwrap(), None);

        db.set_default_provider("gmail".to_string()).await.unwrap();
        assert_eq!(
            db.get_default_provider_opt().await.unwrap(),
            Some("gmail".to_string())
        );

        // INSERT OR REPLACE overwrites the previous default.
        db.set_default_provider("outlook".to_string())
            .await
            .unwrap();
        assert_eq!(
            db.get_default_provider_opt().await.unwrap(),
            Some("outlook".to_string())
        );
    }
}
