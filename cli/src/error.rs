use rusoto;
use monotone;
use log;
use clap;
use serde_json;

error_chain! {
    foreign_links {
        Tls(rusoto::TlsError);
        Credentials(rusoto::CredentialsError);
        Logger(log::SetLoggerError);
        ParseRegion(rusoto::ParseRegionError);
        Clap(clap::Error);
        Json(serde_json::Error);
    }

    links {
        MonotoneAws(monotone::aws::error::Error, monotone::aws::error::ErrorKind);
    }

    errors {
        MissingArgument(a: String) {
            description("missing argument")
            display("missing argument: {}", a)
        }

        InvalidTag(t: String) {
            description("invalid tag")
            display("invalid tag: {}", t)
        }
    }
}