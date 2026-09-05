use std::time::{Duration, SystemTime};

pub type Result<T> = std::result::Result<T, Error>;

/// Errors contain no raw response bodies, headers, URLs, or transport messages.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid configuration or input: {0}")]
    InvalidInput(&'static str),
    #[error("Call login() before using an authenticated endpoint")]
    NotAuthenticated,
    #[error("Log out before signing in again")]
    AlreadyAuthenticated,
    #[error("API returned HTTP {status}; Retry-After: {retry_after:?}")]
    Http {
        status: u16,
        retry_after: Option<Duration>,
    },
    #[error("Network request failed or timed out")]
    Network,
    #[error("API returned malformed or inconsistent data")]
    MalformedResponse,
    #[error(
        "Submission outcome is uncertain (HTTP {status:?}); the job may exist. Do not repeat POST /v1/jobs automatically"
    )]
    SubmissionUncertain { status: Option<u16> },
    #[error(
        "Local wait timed out for job {job_id}; the server job was not cancelled. Resume polling this ID"
    )]
    WaitTimeout { job_id: String },
    #[error("Local file operation failed")]
    FileIo,
    #[error("Unsafe response download: {0}")]
    DownloadValidation(&'static str),
    #[error("Interrupted; any submitted server job continues")]
    Interrupted,
}

impl Error {
    pub(crate) fn retryable_poll(&self) -> bool {
        matches!(self, Self::Network | Self::MalformedResponse)
            || matches!(
                self,
                Self::Http {
                    status: 408 | 409 | 429 | 500..=599,
                    ..
                }
            )
    }
}

/// Parse delta-seconds or an HTTP date, with a supplied clock for deterministic tests.
pub fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    if !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit()) {
        return value.parse::<u64>().ok().map(Duration::from_secs);
    }
    httpdate::parse_http_date(value)
        .ok()
        .map(|date| date.duration_since(now).unwrap_or(Duration::ZERO))
}

pub(crate) fn http_error(response: &reqwest::Response) -> Error {
    Error::Http {
        status: response.status().as_u16(),
        retry_after: response
            .headers()
            .get("Retry-After")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| parse_retry_after(v, SystemTime::now())),
    }
}
