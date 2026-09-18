-- Create emails table
CREATE TABLE emails 
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
);

-- Create sender_email_stats table
CREATE TABLE sender_email_stats
(
    sender TEXT PRIMARY KEY,
    total_emails INTEGER,
    read_emails INTEGER,
    unread_emails INTEGER,
    attachment_count INTEGER,
    no_attachment_count INTEGER
);
