-- Create emails table to store all emails
-- uid is only unique within a single mailbox (label), not across mailboxes, so the primary key
-- must be the (uid, label) pair rather than uid alone. Otherwise a UID collision between two
-- different mailboxes (e.g. INBOX and [Gmail]/Sent Mail) would cause one of them to be silently
-- dropped by INSERT OR IGNORE.
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
    no_attachment_count INTEGER
);

-- Create receiver_email_stats table to store stats for each receiver
CREATE TABLE receiver_email_stats
(
    receiver TEXT PRIMARY KEY,
    total_emails INTEGER,
    read_emails INTEGER,
    unread_emails INTEGER,
    attachment_count INTEGER,
    no_attachment_count INTEGER
);