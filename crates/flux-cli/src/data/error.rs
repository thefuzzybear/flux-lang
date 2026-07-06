/// Errors that can occur during data fetching.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("connection failed to {host}: {reason}")]
    Connection { host: String, reason: String },

    #[error("HTTP {status}: {message}")]
    HttpError { status: u16, message: String },

    #[error("request timed out after {seconds}s — try again later")]
    Timeout { seconds: u64 },

    #[error("rate limited (HTTP 429) — wait before retrying")]
    RateLimited,

    #[error("failed to parse response from {provider}: {detail}")]
    ParseError { provider: String, detail: String },

    #[error("authentication failed for {provider}: {detail}")]
    AuthError { provider: String, detail: String },

    #[error("{0}")]
    Other(String),
}
