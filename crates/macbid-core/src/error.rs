//! Error type shared by the client.

/// Errors surfaced by [`crate::MacBidClient`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Transport-level failure (DNS, TLS, timeout, connection reset…).
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    /// Response body was not the JSON we expected.
    #[error("could not decode response: {0}")]
    Json(#[from] serde_json::Error),

    /// Typesense (or the site API) answered with an error status.
    #[error("api error {status} from {url}: {message}")]
    Api {
        status: u16,
        url: String,
        message: String,
    },

    /// The requested object does not exist.
    #[error("not found: {0}")]
    NotFound(String),

    /// Caller passed something the API cannot express.
    #[error("{0}")]
    Invalid(String),
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, Error>;
