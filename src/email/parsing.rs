use imap::types::Fetch;
use imap_proto::types::{BodyStructure, ContentDisposition};
use tokio_imap::types::{Address, Envelope};

/// Extract subject field from envelope
pub(crate) fn get_subject(envelope: &Envelope<'_>) -> String {
    envelope
        .subject
        .and_then(|subject| std::str::from_utf8(subject).ok())
        .unwrap_or("<no subject>")
        .to_string()
}

/// Extract sender/from field from envelope
pub(crate) fn get_sender(envelope: &Envelope<'_>) -> String {
    envelope
        .from
        .as_ref()
        .and_then(|addresses| addresses.first())
        .map(format_address)
        .unwrap_or_else(|| "<unknown sender>".to_string())
}

/// Extract receiver from envelope
pub(crate) fn get_receiver(envelope: &Envelope<'_>) -> String {
    envelope
        .to
        .as_ref()
        .and_then(|addresses| addresses.first())
        .map(format_address)
        .unwrap_or_else(|| "<unknown receiver>".to_string())
}

/// Extract datetime from envelope
pub(crate) fn get_datetime(envelope: &Envelope<'_>) -> String {
    envelope
        .date
        .and_then(|date| std::str::from_utf8(date).ok())
        .unwrap_or("<unknown date>")
        .to_string()
}

/// Extract is_seen flag from message
pub(crate) fn is_seen(message: &Fetch) -> bool {
    message.flags().contains(&imap::types::Flag::Seen)
}

/// Format email address
pub(crate) fn format_address(address: &Address) -> String {
    let mailbox = address
        .mailbox
        .and_then(|mailbox| std::str::from_utf8(mailbox).ok())
        .unwrap_or("<unknown>");

    let host = address
        .host
        .and_then(|host| std::str::from_utf8(host).ok())
        .unwrap_or("<unknown>");

    format!("{mailbox}@{host}")
}

/// Check if a `BODYSTRUCTURE` (or any of its subparts, for multipart/message messages) has an
/// attachment, by inspecting each part's `Content-Disposition`.
pub(crate) fn body_structure_has_attachment(body_structure: &BodyStructure) -> bool {
    let is_attachment = |disposition: &Option<ContentDisposition>| {
        disposition
            .as_ref()
            .map(|disposition| disposition.ty.eq_ignore_ascii_case("attachment"))
            .unwrap_or(false)
    };

    match body_structure {
        BodyStructure::Basic { common, .. } | BodyStructure::Text { common, .. } => {
            is_attachment(&common.disposition)
        }
        BodyStructure::Message { common, body, .. } => {
            is_attachment(&common.disposition) || body_structure_has_attachment(body)
        }
        BodyStructure::Multipart { common, bodies, .. } => {
            is_attachment(&common.disposition) || bodies.iter().any(body_structure_has_attachment)
        }
    }
}
