[![CI](https://github.com/vbmade2000/emailyzer/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/vbmade2000/emailyzer/actions/workflows/ci.yml)

# emailyzer
Command-line email analyzer. Shows Sender and Receivers statistics.

## User guide
See [guide.md](guide.md) for detailed documentation with command examples, including `providers`, `sync`, `senders`, and `receivers`, as well as known limitations.

## Database migrations
Database migrations are automatically applied when you run the application for the first time. If you want to add a new migration, run the following commands:
```
$ sqlx migrate add -r <migration-name>
$ sqlx migrate run
```

## Change the log level for application
```
$ RUST_LOG=debug ./emailyzer
```