# emailyzer
Simple yet effective email analyzer

## Install dependencies
```
$ sudo apt update 
$ sudo apt install pkg-config libssl-dev
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

## Further reading
[https://datatracker.ietf.org/doc/html/rfc3501](https://datatracker.ietf.org/doc/html/rfc3501)

Check section https://datatracker.ietf.org/doc/html/rfc3501#section-7.4.2