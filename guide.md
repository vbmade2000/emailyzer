# emailyzer — User Guide

`emailyzer` is a command-line email analysis tool. It connects to your email
account over IMAP, stores message metadata locally in SQLite (`emailyzer.db`),
and produces sender/receiver statistics (total, read/unread, with/without
attachments).

Typical workflow:

```bash
emailyzer providers mailboxes ...   # discover correct label names
emailyzer providers add ...         # register your account
emailyzer providers default ...     # (optional) set a default account
emailyzer sync ...                  # fetch emails into the local database
emailyzer senders --refresh ...     # build and view sender stats
emailyzer receivers --refresh ...   # build and view receiver stats
```

All examples below assume the built binary is called `./emailyzer`
(or `cargo run -- <args>` when running from source).

---

## 1. Prerequisites and installation

> **Supported OS:** Linux only. Windows is not supported.

1. Install system dependencies (Debian/Ubuntu):

   ```bash
   sudo apt update
   sudo apt install pkg-config libssl-dev libsqlite3-dev
   ```

2. Clone and build:

   ```bash
   git clone https://github.com/vbmade2000/emailyzer.git
   cd emailyzer
   cargo build --release
   ./target/release/emailyzer --help
   ```

3. Database migrations are applied automatically on first run. The SQLite
   file `emailyzer.db` is created in the current working directory. You only
   need `sqlx-cli` if you are developing new migrations:

   ```bash
   cargo install sqlx-cli --no-default-features --features native-tls,sqlite
   sqlx migrate add -r <migration-name>
   sqlx migrate run
   ```

4. For Gmail you need an [App Password](https://support.google.com/accounts/answer/185833).
   Store it in a file (or pipe it via stdin) — never pass it directly on the
   command line:

   ```bash
   echo -n "xxxx xxxx xxxx xxxx" > /tmp/gmail-app-password.txt
   chmod 600 /tmp/gmail-app-password.txt
   ```

   The `--password-file` / `-w` flag used below accepts either a path or `-`
   to read the secret from stdin.

---

## 2. Managing providers

A *provider* is one email account (IMAP server + credentials + label names).
All `sync`, `senders`, and `receivers` commands operate on a single provider.

### 2.1 Discover mailbox/label names

Different servers name folders differently (e.g. Gmail uses `INBOX` and
`[Gmail]/Sent Mail`). List what the server actually exposes before adding
a provider:

```bash
./emailyzer providers mailboxes \
  --url imap.gmail.com \
  --port 993 \
  --username "you@gmail.com" \
  --password-file /tmp/gmail-app-password.txt
```

Read the password from stdin instead of a file:

```bash
cat /tmp/gmail-app-password.txt | ./emailyzer providers mailboxes \
  --url imap.gmail.com \
  --port 993 \
  --username "you@gmail.com" \
  --password-file -
```

Output is a table of mailbox/label names, e.g. `INBOX`, `[Gmail]/Sent Mail`,
`[Gmail]/Drafts`, etc. Note the exact spelling — you will need it for
`--inbox-label` and `--sent-label` below.

### 2.2 Add a provider

```bash
./emailyzer providers add \
  --name gmail \
  --url imap.gmail.com \
  --port 993 \
  --username "you@gmail.com" \
  --password-file /tmp/gmail-app-password.txt \
  --inbox-label "INBOX" \
  --sent-label "[Gmail]/Sent Mail"
```

| Flag | Meaning |
| ---- | ------- |
| `--name` | Local nickname for this account (used by `--provider`). |
| `--url` / `--port` | IMAP host and port (`imap.gmail.com`, `993` for Gmail). |
| `--username` | Login name, usually your email address. |
| `--password-file` | File containing the password/app-password, or `-` for stdin. |
| `--inbox-label` | Mailbox used as the inbox (e.g. `INBOX`). |
| `--sent-label` | Mailbox used for sent mail (e.g. `[Gmail]/Sent Mail`). |

The password is stored in the OS keyring (keyed by the provider's database
id); the remaining fields are stored in SQLite.

### 2.3 List providers

```bash
./emailyzer providers list
```

Shows name, URL, port, username, inbox/sent labels, and which provider is
the default.

### 2.4 Set a default provider

Once set, you can omit `--provider` from `sync`, `senders`, and `receivers`.

```bash
./emailyzer providers default --name gmail
```

### 2.5 Delete a provider

```bash
./emailyzer providers delete --name gmail
```

You will be asked to confirm. This also permanently deletes all locally
stored emails and stats for that provider (via `ON DELETE CASCADE`).

Skip the prompt (useful in scripts):

```bash
./emailyzer providers delete --name gmail --force
```

### 2.6 How provider passwords are stored

The provider password (e.g. a Gmail App Password) is **never stored in the
SQLite database**. Only the non-secret metadata — name, IMAP host/port,
username, inbox/sent labels — goes into the `providers` table. The password
itself is stored in the **OS keyring** (via the `keyring` crate) under:

- service name: `emailyzer`
- account: the provider's database `id` (a stable number assigned at insert,
  not the user-supplied `--name`, so renaming concerns never orphan the
  stored secret)

How it flows through each command:

| Command | Password handling |
| ------- | ----------------- |
| `providers add` | Reads the secret from `--password-file` (or stdin with `-`, trailing newlines trimmed, empty secrets rejected), inserts the provider row, then saves the password to the keyring. If the keyring write fails, the database row is rolled back so no password-less provider is left behind. |
| `sync` / `senders` / `receivers` | Looks up the provider's `id` and reads the password back from the keyring for IMAP login. Nothing is written to disk. |
| `providers delete` | Deletes the provider row (cascading to its emails/stats) and removes the matching keyring entry. Keyring cleanup failure is logged as a warning but does not fail the delete. |

Practical implications:

- On Linux the keyring backend needs a running credential store (e.g. GNOME
  Keyring via the D-Bus secret service). On a headless machine with no
  secret service, `providers add` will fail when storing the password — run
  `emailyzer` in a desktop session or otherwise provide a secret-service
  implementation.
- Backing up or deleting `emailyzer.db` does not back up or remove passwords;
  they live in the keyring. Moving to another machine requires re-adding
  providers (and their passwords) there.
- If `sync` reports “No stored password found for this provider”, delete and
  re-add the provider to store its password again.

---

## 3. Syncing emails (`sync`)

Fetch message envelopes/flags from the server into the local database:

```bash
# Sync the default provider
./emailyzer sync

# Sync a specific provider
./emailyzer sync --provider gmail
```

What happens on each run:

1. Opens one IMAP connection per mailbox (inbox + sent) plus parallel
   connections for attachment (`BODYSTRUCTURE`) detection.
2. Runs `UID FETCH 1:* (UID FLAGS ENVELOPE)` on each mailbox and inserts
   each message as one row in the `emails` table (`INSERT OR IGNORE`, so
   already-stored UIDs are skipped).
3. Prints `Sync completed successfully` when all fetch and writer tasks finish.

Check progress in the log file (`emailyzer.log` by default, see §5).

### Limitations of `sync` — please read

1. **Every `sync` re-fetches all emails from scratch.**
   There is currently no incremental/delta sync (no `UIDVALIDITY` tracking,
   no “fetch only UIDs greater than the stored maximum”, no `SINCE`/`UNSEEN`
   filter). Each invocation issues a full `UID FETCH 1:*` for both mailboxes
   and walks every message again, even if nothing changed since the last run.
   Duplicates are avoided in the database via `INSERT OR IGNORE`, so results
   stay correct, but:
   - sync time and network traffic grow linearly with mailbox size,
   - large accounts (tens of thousands of messages) can take a long time on
     every run,
   - attachment detection (`BODYSTRUCTURE` fetches in batches of 1500 UIDs
     over 4 parallel connections per mailbox) is repeated for messages that
     were already classified.

   Practical advice: run `sync` infrequently (e.g. once per day) rather than
   before every analysis command.

2. **Only `Inbox` and `Sent` labels are synced; all other labels are ignored.**
   `sync` spawns exactly two fetch tasks: one for the provider's configured
   `--inbox-label` and one for its `--sent-label`. Any other mailbox/label —
   Drafts, Spam, Trash, Starred, custom Gmail labels, archive folders, etc. —
   is never fetched, never stored, and never appears in statistics, even
   though `providers mailboxes` lists them. Likewise, `senders --refresh`
   reads only the inbox label and `receivers --refresh` reads only the sent
   label. If you need another folder analyzed, there is currently no flag to
   select it — this is a planned enhancement.

---

## 4. Analyzing emails

Analysis is a two-step process: `sync` populates the raw `emails` table,
and `senders`/`receivers` aggregate it into stats tables. The `--refresh`
flag rebuilds the stats from the stored emails; without it, the previously
computed stats are displayed.

### 4.1 Senders — who emails you most?

Aggregates the **inbox** label by sender address. Columns: sender, total
emails, read, unread, with attachment, without attachment.

```bash
# First run (or after a new sync): rebuild stats from stored inbox emails
./emailyzer senders --refresh

# Reuse previously computed stats (fast, no recomputation)
./emailyzer senders

# Specific provider
./emailyzer senders --refresh --provider gmail

# Top 10 senders by email count
./emailyzer senders --refresh --top 10

# Sort alphabetically by sender address
./emailyzer senders --refresh --sort-by sender

# Sort by email count (highest first)
./emailyzer senders --refresh --sort-by emails

# Show only selected senders (repeatable flag)
./emailyzer senders --refresh \
  --sender "newsletter@example.com" \
  --sender "boss@example.com"

# Combine filters: top 5 among all, then display
./emailyzer senders --refresh --sort-by emails --top 5
```

Note: `--top N` truncates *after* sorting. If you pass `--top` without
`--sort-by`, results are implicitly sorted by email count descending so
“top N” is well-defined.

### 4.2 Receivers — who do you email most?

Aggregates the **sent** label by receiver address. Flags mirror `senders`,
except the filter flag is `--receiver` / `-e`.

```bash
# First run (or after a new sync): rebuild stats from stored sent emails
./emailyzer receivers --refresh

# Reuse previously computed stats
./emailyzer receivers

# Specific provider
./emailyzer receivers --refresh --provider gmail

# Top 10 receivers
./emailyzer receivers --refresh --top 10

# Sort by email count or alphabetically
./emailyzer receivers --refresh --sort-by emails
./emailyzer receivers --refresh --sort-by receiver

# Show only selected receivers (repeatable flag)
./emailyzer receivers --refresh \
  --receiver "friend@example.com" \
  --receiver "team@example.com"
```

### When to use `--refresh`

| Situation | Command |
| --------- | ------- |
| Just ran `sync` and want up-to-date numbers | Add `--refresh` |
| No stats computed yet (empty output + hint) | Add `--refresh` |
| Only changing display options (`--top`, `--sort-by`, `--sender`) on current data | Omit `--refresh` (faster) |

---

## 5. Logging and debugging

- Logs go to `emailyzer.log` in the current directory by default. Override
  per invocation (global flag, works with every subcommand):

  ```bash
  ./emailyzer --log-file /tmp/emailyzer.log sync --provider gmail
  ```

- Control verbosity with `RUST_LOG` (default `INFO`):

  ```bash
  RUST_LOG=debug ./emailyzer sync --provider gmail
  RUST_LOG=warn ./emailyzer senders --top 5
  ```

- IMAP sockets have a 60-second read/write timeout so a stalled server
  surfaces as an error instead of hanging forever.

---

## 6. Data model (quick reference)

- SQLite file: `emailyzer.db` in the directory you run `emailyzer` from.
- `providers` — one row per `providers add` (name, IMAP host/port, username,
  inbox/sent labels).
- `emails` — one row per fetched message (`uid`, subject, sender, receiver,
  read status, attachment flag `1`/`0`/`-1` for unknown, timestamp, label,
  provider). The label is stored lowercased.
- `sender_email_stats` / `receiver_email_stats` — aggregates rebuilt by
  `--refresh`, scoped per provider.
- Deleting a provider cascades to its emails and stats.

---

## 7. Known limitations and troubleshooting

1. **Full re-fetch on every `sync`** (see §3): no incremental sync yet.
   Expect repeated, slow syncs on large mailboxes.
2. **Inbox + Sent only** (see §3): other labels/folders are listed by
   `providers mailboxes` but never synced or analyzed.
3. **Gmail-only tested.** Other IMAP servers should work if you supply the
   correct `--url`, `--port`, and label names, but sent-label conventions
   differ per server.
4. **Attachment status `-1` means “unknown”**: if a `BODYSTRUCTURE` batch
   fetch fails to parse, that batch is marked unknown rather than aborting
   the sync.
5. **`senders`/`receivers` without `--refresh` show stale data** after a new
   `sync` — re-run with `--refresh` to rebuild.
6. **“No provider specified” error**: pass `--provider <NAME>` or set one
   with `emailyzer providers default --name <NAME>`.
7. **Empty secret error**: the file given to `--password-file` must exist and
   be non-empty (trailing newlines are trimmed); use `-` to read from stdin.
8. **Linux only — Windows is not supported**: `emailyzer` is currently
   supported on Linux only. Running on Windows is not supported
   (use a Linux machine or VM instead).

---

## 8. gRPC server

`emailyzer` also exposes a gRPC server (see `src/grpc/grpc_server.rs`) that
currently serves the `emailyzer.stats.Stats` service on `0.0.0.0:8080`, with
server reflection enabled.

### 8.1 Testing with grpcurl

Because reflection is enabled, you don't need a copy of `proto/stats.proto`
to explore or call the API — [`grpcurl`](https://grpcurl.org/) can discover
everything at runtime. Install it from the [grpcurl releases page](https://github.com/fullstorydev/grpcurl#installation)
or your package manager, then:

```bash
# List all exposed services
grpcurl -plaintext 127.0.0.1:8080 list

# Describe a service's methods and message types
grpcurl -plaintext 127.0.0.1:8080 describe emailyzer.stats.Stats

# Invoke a method (note the Service/Method path, separated by `/`)
grpcurl -plaintext -d '{}' 127.0.0.1:8080 emailyzer.stats.Stats/GetSenderStats
grpcurl -plaintext -d '{}' 127.0.0.1:8080 emailyzer.stats.Stats/GetReceiverStats
```

### 8.2 Pagination

`GetSenderStats` and `GetReceiverStats` results are paginated. `SenderStatsRequest` /
`ReceiverStatsRequest` take two
optional fields:

- `page_size` — max number of rows to return (defaults to 50 if omitted or 0).
- `cursor` — opaque cursor from a previous response's `next_cursor`; omit it
  (or pass an empty string) to fetch the first page.

Each `SenderStatsResponse` / `ReceiverStatsResponse` includes a `next_cursor` field. If it's non-empty,
pass it as `cursor` in the next request to fetch the following page; an empty
`next_cursor` means there are no more results.

```bash
# First page (defaults to page_size 50)
grpcurl -plaintext -d '{"page_size": 10}' \
  127.0.0.1:8080 emailyzer.stats.Stats/GetSenderStats
grpcurl -plaintext -d '{"page_size": 10}' \
  127.0.0.1:8080 emailyzer.stats.Stats/GetReceiverStats

# Subsequent page, using the next_cursor from the previous response
grpcurl -plaintext -d '{"page_size": 10, "cursor": "sender@example.com"}' \
  127.0.0.1:8080 emailyzer.stats.Stats/GetSenderStats
grpcurl -plaintext -d '{"page_size": 10, "cursor": "receiver@example.com"}' \
  127.0.0.1:8080 emailyzer.stats.Stats/GetReceiverStats
```

---

## 9. Running unit tests

Unit tests live next to the source code in `src/**/tests.rs` (database,
providers, email parsing, password store). They use an in-memory SQLite
database and a mock credential store, so they need no network access, no
real email account, and no credentials.

Run the full suite from the project root:

```bash
cargo test
```

This is the same command CI runs (on Ubuntu/Linux, per the Linux-only
support noted in §1):

```bash
cargo test --all-features --verbose
```

Useful variations:

```bash
# Run tests for one module only
cargo test database
cargo test providers

# Run a single test by name, showing its output
cargo test set_default_provider_success_updates_setting -- --nocapture

# Check formatting and lints (also enforced by CI)
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

Tests create no files: unlike the application itself (which writes
`emailyzer.db` and `emailyzer.log` into the working directory), the test
suite runs fully in memory.

---

## 10. Further reading

- IMAP protocol (RFC 3501): <https://datatracker.ietf.org/doc/html/rfc3501>,
  particularly [§7.4.2 on FETCH responses](https://datatracker.ietf.org/doc/html/rfc3501#section-7.4.2).
- Project README (`README.md`) for build prerequisites and migration notes.
