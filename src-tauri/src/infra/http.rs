//! Provider retry header parsing shared by metadata and fingerprint requests.
pub(crate) fn parse_retry_after(value: &str, now: i64) -> Option<u64> {
    value.parse::<u64>().ok().or_else(|| {
        time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc2822)
            .ok()
            .map(|date| date.unix_timestamp().saturating_sub(now).max(0) as u64)
    })
}

pub(crate) fn retry_after_seconds(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            parse_retry_after(value, time::OffsetDateTime::now_utc().unix_timestamp())
        })
}
