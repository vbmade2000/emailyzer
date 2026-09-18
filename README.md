# emailyzer
Simple yet effective email analyzer

## Install dependencies
```
$ sudo apt update 
$ sudo apt install pkg-config libssl-dev libsqlite3-dev
$ cargo install sqlx-cli --no-default-features --features native-tls,sqlite

```

## Export Gmail credentials
```
$ export GMAIL_USERNAME="your-gmail-email"
$ export GMAIL_PWD="your-temporary-gmail-app-password"
```

## Build the application
```
$ git clone https://github.com/vbmade2000/emailyzer.git
$ cd emailyzer
$ cargo build --release
```

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

## Further reading
[https://datatracker.ietf.org/doc/html/rfc3501](https://datatracker.ietf.org/doc/html/rfc3501)

Check section https://datatracker.ietf.org/doc/html/rfc3501#section-7.4.2