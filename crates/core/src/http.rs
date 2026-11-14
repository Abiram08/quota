//! One small blocking HTTPS helper (ureq + Windows SChannel). Errors are
//! mapped to short, secret-free [`ProviderError`]s: neither request headers
//! nor response bodies are ever included in an error message.

use crate::ProviderError;
use serde_json::Value;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_BODY: u64 = 2 * 1024 * 1024;
pub const USER_AGENT: &str = concat!("quota/", env!("CARGO_PKG_VERSION"));

fn agent() -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
    // SChannel with the Windows certificate store. ureq's default (WebPki)
    // would need the bundled Mozilla roots, which this build does not include.
    let tls = TlsConfig::builder()
        .provider(TlsProvider::NativeTls)
        .root_certs(RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .user_agent(USER_AGENT)
        .tls_config(tls)
        .build()
        .into()
}

/// GET returning parsed JSON.
pub fn get_json(url: &str, headers: &[(&str, &str)]) -> Result<Value, ProviderError> {
    let mut req = agent().get(url).header("Accept", "application/json");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    finish(req.call())
}

/// POST of a JSON body returning parsed JSON.
pub fn post_json(url: &str, headers: &[(&str, &str)], body: &str) -> Result<Value, ProviderError> {
    let mut req = agent()
        .post(url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    finish(req.send(body))
}

fn finish(res: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value, ProviderError> {
    let mut resp = res.map_err(transport_error)?;
    let status = resp.status().as_u16();
    if let Some(e) = status_error(status) {
        return Err(e);
    }
    let text = resp
        .body_mut()
        .with_config()
        .limit(MAX_BODY)
        .read_to_string()
        .map_err(|_| ProviderError::Network("could not read response".into()))?;
    serde_json::from_str(&text).map_err(|_| ProviderError::Parse("response is not JSON".into()))
}

/// Maps non-2xx statuses; `None` for success.
pub fn status_error(status: u16) -> Option<ProviderError> {
    Some(match status {
        200..=299 => return None,
        401 => ProviderError::Auth("signed out or token expired".into()),
        403 => ProviderError::Auth("access denied (HTTP 403)".into()),
        404 => ProviderError::Unavailable("usage endpoint not found (HTTP 404); it may have changed".into()),
        429 => ProviderError::Network("rate limited (HTTP 429)".into()),
        s @ 500..=599 => ProviderError::Network(format!("server error (HTTP {s})")),
        s => ProviderError::Network(format!("unexpected HTTP {s}")),
    })
}

fn transport_error(e: ureq::Error) -> ProviderError {
    use ureq::Error as E;
    let msg = match e {
        E::Timeout(_) => "request timed out".to_string(),
        E::HostNotFound => "offline or DNS failure".to_string(),
        E::ConnectionFailed => "connection failed".to_string(),
        E::Io(ref io) => format!("network error ({:?})", io.kind()),
        // ureq error texts carry no request headers, so no secrets.
        ref other => format!("network error ({other})"),
    };
    ProviderError::Network(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses() {
        assert!(status_error(200).is_none());
        assert_eq!(status_error(401).unwrap().kind(), "auth");
        assert_eq!(status_error(429).unwrap().kind(), "network");
        assert_eq!(status_error(503).unwrap().message(), "server error (HTTP 503)");
    }
}
