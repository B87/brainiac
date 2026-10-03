//! The one HTTP client for GitHub and Bitbucket Cloud (docs/architecture.md,
//! Pull requests — v0.3). HTTPS only, the macOS trust store, a time limit on
//! every request. Tokens travel in headers, so no error or log line built
//! from a request (its URL, its status) can contain one.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};

use super::keychain::Token;
use crate::models::{AppError, AppResult, ErrorCode};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// How a request proves who it is from.
pub enum Auth<'a> {
    /// GitHub: `Authorization: Bearer <token>`.
    Bearer(&'a Token),
    /// Bitbucket Cloud API tokens: HTTP Basic with the account's email.
    Basic { user: &'a str, token: &'a Token },
}

/// A provider's answer: any status, the headers, and the body.
pub struct Response {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    pub fn json<T: serde::de::DeserializeOwned>(&self, provider: &str) -> AppResult<T> {
        serde_json::from_slice(&self.body).map_err(|e| {
            AppError::dependency(format!("{provider} sent an answer Brainiac cannot read."))
                .with_details(e.to_string())
        })
    }
}

#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
}

impl Http {
    /// A client that refuses plain `http://`, the one Brainiac uses.
    pub fn new() -> AppResult<Self> {
        Self::build(true)
    }

    /// A client that also speaks plain HTTP, for tests against a local server.
    pub fn insecure_for_tests() -> AppResult<Self> {
        Self::build(false)
    }

    fn build(https_only: bool) -> AppResult<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(concat!("Brainiac/", env!("CARGO_PKG_VERSION"))),
        );
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .https_only(https_only)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| {
                AppError::dependency("Brainiac cannot make network requests.")
                    .with_details(e.to_string())
            })?;
        Ok(Http { client })
    }

    /// `GET url`, with extra headers. Any status is returned; only a request
    /// that got no answer is an error.
    pub async fn get(
        &self,
        provider: &str,
        url: &str,
        auth: Auth<'_>,
        headers: &[(&'static str, &str)],
    ) -> AppResult<Response> {
        let mut request = self.client.get(url).header(ACCEPT, "application/json");
        request = match auth {
            Auth::Bearer(token) => request.header(AUTHORIZATION, bearer(token)?),
            Auth::Basic { user, token } => request.basic_auth(user, Some(token.expose())),
        };
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = request
            .send()
            .await
            .map_err(|e| unreachable(provider, &e))?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response
            .bytes()
            .await
            .map_err(|e| unreachable(provider, &e))?
            .to_vec();
        Ok(Response {
            status,
            headers,
            body,
        })
    }
}

fn bearer(token: &Token) -> AppResult<HeaderValue> {
    let mut value = HeaderValue::from_str(&format!("Bearer {}", token.expose()))
        .map_err(|_| AppError::validation("The token has characters a token cannot have."))?;
    // Keeps the header out of HTTP/2 header compression tables and debug output.
    value.set_sensitive(true);
    Ok(value)
}

/// A request that got no answer: no network, a name that does not resolve, a
/// refused connection, or no answer in time.
fn unreachable(provider: &str, e: &reqwest::Error) -> AppError {
    // The URL in it is safe to show: tokens are only ever in headers.
    let details = format!("{e}");
    if e.is_timeout() {
        AppError::new(
            ErrorCode::Timeout,
            format!("{provider} did not answer in time."),
        )
        .with_details(details)
    } else {
        AppError::dependency(format!(
            "Cannot reach {provider}. Check the network connection."
        ))
        .with_details(details)
    }
}

/// An answer no caller expected: the provider failing, out of requests, or a
/// status Brainiac does not handle there.
pub fn unexpected(provider: &str, response: &Response) -> AppError {
    let status = response.status;
    let details = format!(
        "HTTP {status}: {}",
        String::from_utf8_lossy(&response.body[..response.body.len().min(500)])
    );
    let message = match status {
        429 => format!("{provider} has no requests left for this account right now."),
        500..=599 => format!("{provider} is not working right now (HTTP {status})."),
        _ => format!("{provider} answered with an unexpected HTTP {status}."),
    };
    AppError::dependency(message).with_details(details)
}
