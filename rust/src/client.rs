use crate::{Error, Result};
use reqwest::{
    Method, RequestBuilder, Response,
    header::{HeaderMap, HeaderValue},
};
use serde::de::DeserializeOwned;
use std::time::Duration;
use url::Url;

pub const DEFAULT_BASE_URL: &str = "https://ai.openjvspace.com";

#[derive(Clone, Debug)]
pub struct ClientConfig {
    pub base_url: String,
    pub request_timeout: Duration,
    pub poll_interval: Duration,
    pub wait_timeout: Duration,
    /// Stop after this many consecutive transient polling errors (minimum one).
    pub max_poll_errors: u32,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.into(),
            request_timeout: Duration::from_secs(120),
            poll_interval: Duration::from_secs(2),
            wait_timeout: Duration::from_secs(3600),
            max_poll_errors: 8,
        }
    }
}

/// Intentionally does not implement Debug or expose its bearer token.
pub struct JvClient {
    pub(crate) http: reqwest::Client,
    pub(crate) base: Url,
    pub(crate) authorization: Option<HeaderValue>,
    pub(crate) config: ClientConfig,
}

impl JvClient {
    pub fn new(config: ClientConfig) -> Result<Self> {
        let base = validate_base_url(&config.base_url)?;
        for duration in [
            config.request_timeout,
            config.poll_interval,
            config.wait_timeout,
        ] {
            if duration.is_zero() || duration > Duration::from_secs(86400 * 30) {
                return Err(Error::InvalidInput(
                    "timeouts must be positive and at most 30 days",
                ));
            }
        }
        if config.max_poll_errors == 0 {
            return Err(Error::InvalidInput("max_poll_errors must be positive"));
        }
        let mut headers = HeaderMap::new();
        headers.insert("Accept", HeaderValue::from_static("application/json"));
        headers.insert("X-JV-CSRF", HeaderValue::from_static("1"));
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .user_agent("JV-AI-Rust-Example/0.1")
            .timeout(config.request_timeout)
            .connect_timeout(config.request_timeout.min(Duration::from_secs(30)))
            // Even same-origin redirects must not repeat submission or change its method.
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .build()
            .map_err(|_| Error::Network)?;
        Ok(Self {
            http,
            base,
            authorization: None,
            config,
        })
    }

    pub fn is_authenticated(&self) -> bool {
        self.authorization.is_some()
    }

    pub(crate) fn authenticated(&self, method: Method, url: Url) -> Result<RequestBuilder> {
        let token = self.authorization.as_ref().ok_or(Error::NotAuthenticated)?;
        Ok(self
            .http
            .request(method, url)
            .header("Authorization", token.clone()))
    }

    pub(crate) fn endpoint(&self, path: &str) -> Result<Url> {
        self.base
            .join(path)
            .map_err(|_| Error::InvalidInput("invalid endpoint"))
    }
}

/// HTTPS origins, or HTTP on localhost/127.0.0.1 for development, as in Python.
pub fn validate_base_url(value: &str) -> Result<Url> {
    let value = value.trim();
    let invalid = || {
        Error::InvalidInput(
            "base URL must be an HTTPS origin or loopback HTTP origin, with no credentials, path, query or fragment",
        )
    };
    if value.chars().any(char::is_control) || value.contains('\\') || value.contains('@') {
        return Err(invalid());
    }
    let url = Url::parse(value).map_err(|_| invalid())?;
    // Check the raw suffix too: URL parsing can normalize /a/.. into /.
    let authority = value.split_once("://").ok_or_else(invalid)?.1;
    if authority
        .find('/')
        .is_some_and(|i| authority[i..].chars().any(|c| c != '/'))
    {
        return Err(invalid());
    }
    let loopback =
        url.scheme() == "http" && matches!(url.host_str(), Some("127.0.0.1" | "localhost"));
    if (url.scheme() != "https" && !loopback)
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().chars().all(|c| c == '/')
    {
        return Err(invalid());
    }
    let mut url = url;
    url.set_path("/");
    Ok(url)
}

pub(crate) async fn read_json<T: DeserializeOwned>(mut response: Response) -> Result<T> {
    const MAX_JSON_BYTES: usize = 8 * 1024 * 1024;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Network)? {
        if chunk.len() > MAX_JSON_BYTES - body.len() {
            return Err(Error::MalformedResponse);
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| Error::MalformedResponse)
}
