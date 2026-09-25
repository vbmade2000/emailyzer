-- Create emails table to store all emails
-- uid is only unique within a single mailbox (label), not across mailboxes, so the primary key
-- must be the (uid, label) pair rather than uid alone. Otherwise a UID collision between two
-- different mailboxes (e.g. INBOX and [Gmail]/Sent Mail) would cause one of them to be silently
-- dropped by INSERT OR IGNORE.
-- provider_id links each email to the provider it was fetched from (see "providers" table
-- below). ON DELETE CASCADE ensures deleting a provider also deletes all of its emails.
CREATE TABLE emails
(
    uid INTEGER NOT NULL,
    subject TEXT,
    sender TEXT NOT NULL,
    receiver TEXT NOT NULL,
    read_status bool,
    has_attachment SMALLINT NOT NULL,
    timestamp TEXT NOT NULL,
    body TEXT,
    label TEXT NOT NULL DEFAULT 'INBOX',
    provider_id INTEGER NOT NULL REFERENCES providers (id) ON DELETE CASCADE,
    PRIMARY KEY (uid, label)
);

-- Create sender_email_stats table to store stats for each sender
CREATE TABLE sender_email_stats
(
    sender TEXT PRIMARY KEY,
    total_emails INTEGER,
    read_emails INTEGER,
    unread_emails INTEGER,
    attachment_count INTEGER,
    no_attachment_count INTEGER,
    provider_id INTEGER NOT NULL REFERENCES providers (id) ON DELETE CASCADE
);

-- Create receiver_email_stats table to store stats for each receiver
CREATE TABLE receiver_email_stats
(
    receiver TEXT PRIMARY KEY,
    total_emails INTEGER,
    read_emails INTEGER,
    unread_emails INTEGER,
    attachment_count INTEGER,
    no_attachment_count INTEGER,
    provider_id INTEGER NOT NULL REFERENCES providers (id) ON DELETE CASCADE
);

-- Create a provider table to store all the email providers
-- inbox_label/sent_label store the mailbox names for Inbox and Sent Emails, since these
-- differ between IMAP providers (e.g. Gmail uses "[Gmail]/Sent Mail" for sent emails while
-- other providers may use "Sent" or "Sent Items"). Default to Gmail's labels.
-- id is the primary key (rather than provider_name) so it can be used as a stable foreign key
-- from other tables even if a provider is ever renamed.
CREATE TABLE providers
(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_name TEXT NOT NULL UNIQUE,
    imap_server_url TEXT NOT NULL,
    imap_server_port INTEGER NOT NULL,
    username TEXT NOT NULL,
    inbox_label TEXT NOT NULL DEFAULT 'INBOX',
    sent_label TEXT NOT NULL DEFAULT '[Gmail]/Sent Mail'
);

-- Create a table to application wide store settings
CREATE TABLE settings
(
    key TEXT PRIMARY KEY,
    value TEXT
);
