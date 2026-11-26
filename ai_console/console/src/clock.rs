//! RFC 3339 UTC timestamps as unix seconds.

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub fn parse_rfc3339(s: &str) -> Option<i64> {
    OffsetDateTime::parse(s, &Rfc3339)
        .ok()
        .map(|t| t.unix_timestamp())
}

pub fn format_rfc3339(t: i64) -> String {
    OffsetDateTime::from_unix_timestamp(t)
        .ok()
        .and_then(|t| t.format(&Rfc3339).ok())
        .unwrap_or_default()
}

pub fn now_unix() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp()
}
