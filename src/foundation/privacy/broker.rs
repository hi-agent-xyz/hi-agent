use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::Duration;

use anyhow::{Context, bail};
use bytes::BytesMut;
use futures::StreamExt;
use reqwest::header::{HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::PrivacyBoundary;

const MAX_REQUEST_BODY: usize = 2 * 1024 * 1024;
const MAX_RESPONSE_BODY: usize = 2 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
pub struct BrokerResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: String,
    pub truncated: bool,
}

pub async fn http_request(
    privacy: &PrivacyBoundary,
    args: &Value,
) -> anyhow::Result<BrokerResponse> {
    let method = args
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_ascii_uppercase();
    let method = reqwest::Method::from_bytes(method.as_bytes()).context("invalid HTTP method")?;
    if !matches!(
        method,
        reqwest::Method::GET
            | reqwest::Method::POST
            | reqwest::Method::PUT
            | reqwest::Method::PATCH
            | reqwest::Method::DELETE
            | reqwest::Method::HEAD
    ) {
        bail!("HTTP method is not allowed");
    }

    let raw_url = args
        .get("url")
        .and_then(Value::as_str)
        .context("hi_http_request requires `url`")?;
    let url = reqwest::Url::parse(raw_url).context("invalid HTTP URL")?;
    validate_url(&url)?;

    let mut request = privacy
        .http()
        .request(method, url)
        .timeout(Duration::from_secs(60));
    if let Some(headers) = args.get("headers").and_then(Value::as_object) {
        if headers.len() > 64 {
            bail!("too many HTTP headers");
        }
        for (name, value) in headers {
            let name =
                HeaderName::from_bytes(name.as_bytes()).context("invalid HTTP header name")?;
            if is_forbidden_request_header(&name) {
                bail!("HTTP header is managed by the broker");
            }
            let value = value
                .as_str()
                .context("HTTP header values must be strings")
                .and_then(|value| {
                    HeaderValue::from_str(value).context("invalid HTTP header value")
                })?;
            request = request.header(name, value);
        }
    }

    if let Some(secret_ref) = args.get("auth_ref").and_then(Value::as_str) {
        let material = privacy.store().resolve_for_http(secret_ref)?;
        let scheme = args
            .get("auth_scheme")
            .and_then(Value::as_str)
            .unwrap_or("bearer");
        request = match scheme {
            "bearer" => request.bearer_auth(material.expose_to_broker()),
            "basic" => {
                let username = args
                    .get("auth_username")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                request.basic_auth(username, Some(material.expose_to_broker()))
            }
            "header" => {
                let name = args
                    .get("auth_header")
                    .and_then(Value::as_str)
                    .context("header auth requires `auth_header`")?;
                let name =
                    HeaderName::from_bytes(name.as_bytes()).context("invalid auth header name")?;
                if is_forbidden_request_header(&name) {
                    bail!("auth header is managed or forbidden");
                }
                let value = HeaderValue::from_str(material.expose_to_broker())
                    .context("secret is not a valid HTTP header value")?;
                request.header(name, value)
            }
            _ => bail!("auth_scheme must be bearer, basic, or header"),
        };
    }

    if let Some(body) = args.get("body") {
        let bytes = match body {
            Value::String(text) => text.as_bytes().to_vec(),
            other => serde_json::to_vec(other).context("encoding HTTP JSON body")?,
        };
        if bytes.len() > MAX_REQUEST_BODY {
            bail!("HTTP request body exceeds {MAX_REQUEST_BODY} bytes");
        }
        if !body.is_string() {
            request = request.header(reqwest::header::CONTENT_TYPE, "application/json");
        }
        request = request.body(bytes);
    }

    let response = request.send().await.context("HTTP request failed")?;
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .filter(|(name, _)| {
            matches!(
                name.as_str(),
                "content-type" | "etag" | "last-modified" | "location" | "retry-after"
            )
        })
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.to_string(), value.to_string()))
        })
        .collect::<BTreeMap<_, _>>();

    let mut bytes = BytesMut::new();
    let mut truncated = false;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading HTTP response")?;
        let remaining = MAX_RESPONSE_BODY.saturating_sub(bytes.len());
        if chunk.len() > remaining {
            bytes.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    // Returned as it came back. The response to a call the agent chose to make is
    // the agent's own business — only text a person typed is filtered, and only on
    // the way into a session.
    Ok(BrokerResponse {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
        truncated,
    })
}

fn validate_url(url: &reqwest::Url) -> anyhow::Result<()> {
    if !url.username().is_empty() || url.password().is_some() {
        bail!("credentials in URLs are not allowed");
    }
    match url.scheme() {
        "https" => Ok(()),
        "http" if is_loopback(url) => Ok(()),
        "http" => bail!("plain HTTP is allowed only for loopback destinations"),
        _ => bail!("only HTTP(S) URLs are allowed"),
    }
}

fn is_loopback(url: &reqwest::Url) -> bool {
    match url.host_str() {
        Some("localhost") => true,
        Some(host) => host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()),
        None => false,
    }
}

fn is_forbidden_request_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "authorization"
            | "proxy-authorization"
            | "cookie"
            | "host"
            | "content-length"
            | "connection"
            | "transfer-encoding"
    )
}
