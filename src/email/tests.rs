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

    // NOTE: `is_seen`, `sync_emails`, `get_sender_stats`, and `_get_mailboxes`
    // are intentionally not unit-tested here. `is_seen` takes an
    // `imap::types::Fetch` whose flag storage is `pub(crate)` to the `imap`
    // crate, so it cannot be constructed from outside that crate. The others
    // require a live IMAP connection and/or a SQLite database file.
}
