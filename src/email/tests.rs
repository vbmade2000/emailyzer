#[cfg(test)]
mod email_tests {
    use imap_proto::{Address, Envelope};

    use crate::email::imap_client::{get_client, get_tls_connector};
    use crate::email::parsing::{
        format_address, get_datetime, get_receiver, get_sender, get_subject,
    };

    fn make_envelope<'a>(
        subject: Option<&'a [u8]>,
        from: Option<Vec<Address<'a>>>,
        to: Option<Vec<Address<'a>>>,
        date: Option<&'a [u8]>,
    ) -> Envelope<'a> {
        Envelope {
            date,
            subject,
            from,
            sender: None,
            reply_to: None,
            to,
            cc: None,
            bcc: None,
            in_reply_to: None,
            message_id: None,
        }
    }

    fn make_address<'a>(mailbox: Option<&'a [u8]>, host: Option<&'a [u8]>) -> Address<'a> {
        Address {
            name: None,
            adl: None,
            mailbox,
            host,
        }
    }

    // get_subject -----------------------------------------------------------

    #[test]
    fn get_subject_returns_subject_when_present() {
        let envelope = make_envelope(Some(b"Hello world"), None, None, None);
        assert_eq!(get_subject(&envelope), "Hello world");
    }

    #[test]
    fn get_subject_returns_placeholder_when_missing() {
        let envelope = make_envelope(None, None, None, None);
        assert_eq!(get_subject(&envelope), "<no subject>");
    }

    #[test]
    fn get_subject_returns_placeholder_on_invalid_utf8() {
        let envelope = make_envelope(Some(&[0xff, 0xfe]), None, None, None);
        assert_eq!(get_subject(&envelope), "<no subject>");
    }

    #[test]
    fn get_subject_returns_empty_string_when_subject_is_empty() {
        let envelope = make_envelope(Some(b""), None, None, None);
        assert_eq!(get_subject(&envelope), "");
    }

    // get_sender ------------------------------------------------------------

    #[test]
    fn get_sender_returns_formatted_address() {
        let envelope = make_envelope(
            None,
            Some(vec![make_address(Some(b"malhar"), Some(b"example.com"))]),
            None,
            None,
        );
        assert_eq!(get_sender(&envelope), "malhar@example.com");
    }

    #[test]
    fn get_sender_uses_first_address_only() {
        let envelope = make_envelope(
            None,
            Some(vec![
                make_address(Some(b"malhar"), Some(b"example.com")),
                make_address(Some(b"nimesh"), Some(b"example.com")),
            ]),
            None,
            None,
        );
        assert_eq!(get_sender(&envelope), "malhar@example.com");
    }

    #[test]
    fn get_sender_returns_placeholder_when_missing() {
        let envelope = make_envelope(None, None, None, None);
        assert_eq!(get_sender(&envelope), "<unknown sender>");
    }

    #[test]
    fn get_sender_returns_placeholder_when_from_is_empty() {
        let envelope = make_envelope(None, Some(vec![]), None, None);
        assert_eq!(get_sender(&envelope), "<unknown sender>");
    }

    #[test]
    fn get_sender_handles_missing_mailbox_and_host() {
        let envelope = make_envelope(None, Some(vec![make_address(None, None)]), None, None);
        assert_eq!(get_sender(&envelope), "<unknown>@<unknown>");
    }

    #[test]
    fn get_sender_handles_invalid_utf8() {
        let envelope = make_envelope(
            None,
            Some(vec![make_address(Some(&[0xff]), Some(&[0xfe]))]),
            None,
            None,
        );
        assert_eq!(get_sender(&envelope), "<unknown>@<unknown>");
    }

    // get_receiver ----------------------------------------------------------

    #[test]
    fn get_receiver_returns_formatted_address() {
        let envelope = make_envelope(
            None,
            None,
            Some(vec![make_address(Some(b"malhar"), Some(b"example.com"))]),
            None,
        );
        assert_eq!(get_receiver(&envelope), "malhar@example.com");
    }

    #[test]
    fn get_receiver_uses_first_address_only() {
        let envelope = make_envelope(
            None,
            None,
            Some(vec![
                make_address(Some(b"nimesh"), Some(b"example.com")),
                make_address(Some(b"adi"), Some(b"example.com")),
            ]),
            None,
        );
        assert_eq!(get_receiver(&envelope), "nimesh@example.com");
    }

    #[test]
    fn get_receiver_returns_placeholder_when_missing() {
        let envelope = make_envelope(None, None, None, None);
        assert_eq!(get_receiver(&envelope), "<unknown receiver>");
    }

    #[test]
    fn get_receiver_returns_placeholder_when_to_is_empty() {
        let envelope = make_envelope(None, None, Some(vec![]), None);
        assert_eq!(get_receiver(&envelope), "<unknown receiver>");
    }

    #[test]
    fn get_receiver_handles_missing_mailbox_and_host() {
        let envelope = make_envelope(None, None, Some(vec![make_address(None, None)]), None);
        assert_eq!(get_receiver(&envelope), "<unknown>@<unknown>");
    }

    // get_datetime ----------------------------------------------------------

    #[test]
    fn get_datetime_returns_date_when_present() {
        let envelope = make_envelope(None, None, None, Some(b"Mon, 1 Jan 2024 00:00:00 +0000"));
        assert_eq!(get_datetime(&envelope), "Mon, 1 Jan 2024 00:00:00 +0000");
    }

    #[test]
    fn get_datetime_returns_placeholder_when_missing() {
        let envelope = make_envelope(None, None, None, None);
        assert_eq!(get_datetime(&envelope), "<unknown date>");
    }

    #[test]
    fn get_datetime_returns_placeholder_on_invalid_utf8() {
        let envelope = make_envelope(None, None, None, Some(&[0xff, 0xfe]));
        assert_eq!(get_datetime(&envelope), "<unknown date>");
    }

    // format_address --------------------------------------------------------

    #[test]
    fn format_address_combines_mailbox_and_host() {
        let address = make_address(Some(b"malhar"), Some(b"example.com"));
        assert_eq!(format_address(&address), "malhar@example.com");
    }

    #[test]
    fn format_address_uses_unknown_for_missing_parts() {
        let address = make_address(None, None);
        assert_eq!(format_address(&address), "<unknown>@<unknown>");
    }

    #[test]
    fn format_address_uses_unknown_for_invalid_utf8() {
        let address = make_address(Some(&[0xff]), Some(&[0xfe]));
        assert_eq!(format_address(&address), "<unknown>@<unknown>");
    }

    #[test]
    fn format_address_keeps_valid_part_when_other_is_invalid() {
        let address = make_address(Some(b"malhar"), Some(&[0xfe]));
        assert_eq!(format_address(&address), "malhar@<unknown>");

        let address = make_address(Some(&[0xff]), Some(b"example.com"));
        assert_eq!(format_address(&address), "<unknown>@example.com");
    }

    // get_tls_connector -----------------------------------------------------

    #[tokio::test]
    async fn get_tls_connector_builds_successfully() {
        assert!(get_tls_connector().await.is_ok());
    }

    // get_client ------------------------------------------------------------
    // `.invalid` (RFC 2606) never resolves, so this exercises the error path
    // without touching the real Gmail servers.

    #[tokio::test]
    async fn get_client_returns_error_for_unresolvable_host() {
        let result = get_client("invalid.invalid", 993).await;
        assert!(result.is_err());
    }

    // NOTE: `is_seen`, `sync_emails`, and `_get_mailboxes` are intentionally
    // not unit-tested here. `is_seen` takes an `imap::types::Fetch` whose
    // flag storage is `pub(crate)` to the `imap` crate, so it cannot be
    // constructed from outside that crate. The others require a live IMAP
    // connection. `get_sender_stats` is integration-tested (against an
    // in-memory SQLite database) in `stats_tests` below.
}

#[cfg(test)]
mod stats_tests {
    use crate::database::{DatabaseLocation, DatabaseManager};
    use crate::email::stats::get_sender_stats;
    use crate::email::types::Email;
    use crate::providers::Provider;
    use crate::{SenderSortBy, SendersArgs};

    /// Fresh isolated database per test: each in-memory SQLite instance is private to its
    /// single-connection pool and disappears when the manager is dropped, so no locking or
    /// cleanup is needed.
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

    async fn seed_provider(db: &DatabaseManager, name: &str) -> i64 {
        db.create_provider(make_provider(name))
            .await
            .expect("seed provider")
    }

    fn make_email(uid: u32, sender: &str, read: bool, attachment: i16, provider: &str) -> Email {
        Email {
            uid,
            subject: format!("Subject {uid}"),
            sender: sender.to_string(),
            read_status: read,
            receiver: "bob@example.com".to_string(),
            attachment,
            timestamp: "Mon, 1 Jan 2024 00:00:00 +0000".to_string(),
            body: "".to_string(),
            label: "inbox".to_string(),
            provider: provider.to_string(),
        }
    }

    fn senders_args(provider: Option<&str>, refresh: bool) -> SendersArgs {
        SendersArgs {
            refresh,
            sender: Vec::new(),
            sort_by: None,
            top: None,
            provider: provider.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn fails_when_no_provider_specified_and_no_default_set() {
        let db = test_db().await;
        let result = get_sender_stats(senders_args(None, false), &db).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fails_when_provider_not_found() {
        let db = test_db().await;
        let result = get_sender_stats(senders_args(Some("missing"), false), &db).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn refresh_aggregates_emails_and_persists_sender_stats() {
        let db = test_db().await;
        seed_provider(&db, "gmail").await;

        db.create_email_entry(make_email(1, "alice@example.com", true, 1, "gmail"))
            .await
            .unwrap();
        db.create_email_entry(make_email(2, "alice@example.com", false, 0, "gmail"))
            .await
            .unwrap();
        db.create_email_entry(make_email(3, "carol@example.com", true, 0, "gmail"))
            .await
            .unwrap();

        get_sender_stats(senders_args(Some("gmail"), true), &db)
            .await
            .unwrap();

        let stats = db.read_sender_email_stats("gmail".to_string()).await.unwrap();
        assert_eq!(stats.len(), 2);

        let alice = stats
            .iter()
            .find(|s| s.sender == "alice@example.com")
            .expect("alice stats present");
        assert_eq!(alice.total_emails, 2);
        assert_eq!(alice.read_emails, 1);
        assert_eq!(alice.unread_emails, 1);
        assert_eq!(alice.attachment_count, 1);
        assert_eq!(alice.no_attachment_count, 1);

        let carol = stats
            .iter()
            .find(|s| s.sender == "carol@example.com")
            .expect("carol stats present");
        assert_eq!(carol.total_emails, 1);
        assert_eq!(carol.read_emails, 1);
        assert_eq!(carol.no_attachment_count, 1);
    }

    #[tokio::test]
    async fn refresh_overwrites_previously_persisted_stats() {
        let db = test_db().await;
        seed_provider(&db, "gmail").await;

        db.create_email_entry(make_email(1, "alice@example.com", true, 1, "gmail"))
            .await
            .unwrap();
        get_sender_stats(senders_args(Some("gmail"), true), &db)
            .await
            .unwrap();
        assert_eq!(
            db.read_sender_email_stats("gmail".to_string())
                .await
                .unwrap()
                .len(),
            1
        );

        // A second email from a different sender arrives; re-running with --refresh should
        // replace the stale stats rather than accumulate on top of them.
        db.create_email_entry(make_email(2, "dave@example.com", false, 0, "gmail"))
            .await
            .unwrap();
        get_sender_stats(senders_args(Some("gmail"), true), &db)
            .await
            .unwrap();

        let stats = db.read_sender_email_stats("gmail".to_string()).await.unwrap();
        assert_eq!(stats.len(), 2);
    }

    #[tokio::test]
    async fn without_refresh_reads_existing_stats_without_erroring() {
        let db = test_db().await;
        seed_provider(&db, "gmail").await;
        db.create_sender_email_stats_entry(
            "alice@example.com".to_string(),
            5,
            3,
            2,
            1,
            4,
            "gmail".to_string(),
        )
        .await
        .unwrap();

        let result = get_sender_stats(senders_args(Some("gmail"), false), &db).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn without_refresh_and_no_existing_stats_is_ok() {
        let db = test_db().await;
        seed_provider(&db, "gmail").await;

        let result = get_sender_stats(senders_args(Some("gmail"), false), &db).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn uses_default_provider_when_none_specified() {
        let db = test_db().await;
        seed_provider(&db, "gmail").await;
        db.set_default_provider("gmail".to_string()).await.unwrap();
        db.create_email_entry(make_email(1, "alice@example.com", true, 1, "gmail"))
            .await
            .unwrap();

        let result = get_sender_stats(senders_args(None, true), &db).await;
        assert!(result.is_ok());
        assert_eq!(
            db.read_sender_email_stats("gmail".to_string())
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn refresh_scopes_stats_to_requested_provider() {
        let db = test_db().await;
        seed_provider(&db, "gmail").await;
        seed_provider(&db, "outlook").await;

        db.create_email_entry(make_email(1, "alice@example.com", true, 1, "gmail"))
            .await
            .unwrap();
        db.create_email_entry(make_email(2, "eve@example.com", true, 0, "outlook"))
            .await
            .unwrap();

        get_sender_stats(senders_args(Some("gmail"), true), &db)
            .await
            .unwrap();

        assert_eq!(
            db.read_sender_email_stats("gmail".to_string())
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(
            db.read_sender_email_stats("outlook".to_string())
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn works_with_sort_by_and_top_flags_set() {
        let db = test_db().await;
        seed_provider(&db, "gmail").await;
        db.create_email_entry(make_email(1, "alice@example.com", true, 1, "gmail"))
            .await
            .unwrap();
        db.create_email_entry(make_email(2, "bob@example.com", true, 1, "gmail"))
            .await
            .unwrap();

        let mut args = senders_args(Some("gmail"), true);
        args.sort_by = Some(SenderSortBy::Emails);
        args.top = Some(1);

        let result = get_sender_stats(args, &db).await;
        assert!(result.is_ok());
        // Persistence is unaffected by display-only sort/top flags.
        assert_eq!(
            db.read_sender_email_stats("gmail".to_string())
                .await
                .unwrap()
                .len(),
            2
        );
    }
}
