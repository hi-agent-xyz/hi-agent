//! The bucket: Tencent COS's XML API, spoken directly and signed by hand.
//!
//! Five calls — put, and the four of a multipart upload — are all a core ever
//! makes, and each is one signed HTTP request. The SDK would be a large
//! dependency for that, and a second HTTP stack beside reqwest.
//!
//! Signing is COS's own scheme (`q-sign-algorithm=sha1`), not AWS SigV4: a key
//! time window, an HMAC-SHA1 of it, and an HMAC-SHA1 over the method, path, and
//! whichever query parameters and headers are named as signed.

use std::time::Duration;

use base64::Engine as _;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac as _};
use md5::{Digest as _, Md5};
use sha1::Sha1;

/// The write half of what the broker hands a core: a temporary key from STS,
/// narrowed by policy to this core's own prefix of the bucket.
#[derive(Clone)]
pub struct WriteKey {
    pub secret_id: String,
    pub secret_key: String,
    /// Goes on every request as `x-cos-security-token`; a temporary key is not
    /// accepted without it.
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for WriteKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriteKey").field("expires_at", &self.expires_at).finish_non_exhaustive()
    }
}

/// One bucket, reached with one key.
pub struct Client {
    /// `https://<bucket>.cos.<region>.myqcloud.com`; a test points it elsewhere.
    base: String,
    host: String,
    key: WriteKey,
    http: reqwest::Client,
}

/// How an object is to be served back: exactly what the core would have said.
pub struct Meta<'a> {
    pub content_type: &'a str,
    pub cache_control: &'a str,
}

impl Client {
    pub fn new(bucket: &str, region: &str, key: WriteKey) -> Self {
        let host = format!("{bucket}.cos.{region}.myqcloud.com");
        Self::at(format!("https://{host}"), host, key)
    }

    fn at(base: String, host: String, key: WriteKey) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            // One part is at most `PART` bytes. Ten minutes for 8 MB is 14 KB/s —
            // a bad uplink, but one this should outlast rather than abandon.
            .timeout(Duration::from_secs(600))
            .build()
            .unwrap_or_default();
        Self { base, host, key, http }
    }

    /// Put a whole object in one request.
    pub async fn put(&self, key: &str, meta: &Meta<'_>, body: Bytes) -> anyhow::Result<()> {
        let md5 = content_md5(&body);
        let res = self
            .request(reqwest::Method::PUT, key, &[])
            .header("Content-Type", meta.content_type)
            .header("Cache-Control", meta.cache_control)
            .header("Content-MD5", md5)
            .body(body)
            .send()
            .await?;
        ok(res, "put").await.map(drop)
    }

    /// Start a multipart upload; the object's headers are fixed here, not at
    /// completion.
    pub async fn initiate(&self, key: &str, meta: &Meta<'_>) -> anyhow::Result<String> {
        let res = self
            .request(reqwest::Method::POST, key, &[("uploads", "")])
            .header("Content-Type", meta.content_type)
            .header("Cache-Control", meta.cache_control)
            .send()
            .await?;
        let body = ok(res, "initiate").await?;
        element(&body, "UploadId")
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("initiate answered without an UploadId: {body}"))
    }

    /// Upload part `n` (from 1) and return its ETag, which completion must repeat.
    pub async fn upload_part(
        &self,
        key: &str,
        upload_id: &str,
        n: u32,
        body: Bytes,
    ) -> anyhow::Result<String> {
        let md5 = content_md5(&body);
        let n = n.to_string();
        let res = self
            .request(reqwest::Method::PUT, key, &[("partNumber", &n), ("uploadId", upload_id)])
            .header("Content-MD5", md5)
            .body(body)
            .send()
            .await?;
        let etag = res
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        ok(res, "upload part").await?;
        etag.ok_or_else(|| anyhow::anyhow!("part {n} was accepted without an ETag"))
    }

    pub async fn complete(
        &self,
        key: &str,
        upload_id: &str,
        parts: &[(u32, String)],
    ) -> anyhow::Result<()> {
        let mut xml = String::from("<CompleteMultipartUpload>");
        for (n, etag) in parts {
            xml.push_str(&format!("<Part><PartNumber>{n}</PartNumber><ETag>{etag}</ETag></Part>"));
        }
        xml.push_str("</CompleteMultipartUpload>");
        let res = self
            .request(reqwest::Method::POST, key, &[("uploadId", upload_id)])
            .header("Content-Type", "application/xml")
            .body(xml)
            .send()
            .await?;
        // A completion can fail after the status line: the body is where it says so.
        let body = ok(res, "complete").await?;
        anyhow::ensure!(!body.contains("<Error>"), "complete failed: {body}");
        Ok(())
    }

    /// Give up on an upload, so its parts stop being stored. Best effort: the
    /// bucket's lifecycle rule for incomplete uploads is the backstop.
    pub async fn abort(&self, key: &str, upload_id: &str) {
        let res = self.request(reqwest::Method::DELETE, key, &[("uploadId", upload_id)]).send().await;
        if let Err(e) = res {
            tracing::debug!(key, error = %e, "could not abort a multipart upload");
        }
    }

    /// A signed request for `key` (no leading slash) with `params` in the query.
    fn request(
        &self,
        method: reqwest::Method,
        key: &str,
        params: &[(&str, &str)],
    ) -> reqwest::RequestBuilder {
        let path = format!("/{key}");
        let mut url = format!("{}{path}", self.base);
        if !params.is_empty() {
            let q: Vec<String> = params
                .iter()
                .map(|(k, v)| if v.is_empty() { encode(k) } else { format!("{}={}", encode(k), encode(v)) })
                .collect();
            url.push('?');
            url.push_str(&q.join("&"));
        }
        let now = Utc::now().timestamp();
        // The signature's own life, not the key's: long enough to cover one part
        // on a slow link, which is the longest a single request here takes.
        let auth = authorization(
            &self.key.secret_id,
            &self.key.secret_key,
            method.as_str(),
            &path,
            params,
            &[("host", &self.host)],
            now - 60,
            now + 3600,
        );
        let req = self.http.request(method, url).header("Authorization", auth);
        if self.key.token.is_empty() {
            req
        } else {
            req.header("x-cos-security-token", &self.key.token)
        }
    }
}

/// The body of a successful answer, or the error COS gave in its place.
async fn ok(res: reqwest::Response, what: &str) -> anyhow::Result<String> {
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    if status.is_success() {
        return Ok(body);
    }
    let code = element(&body, "Code").unwrap_or("");
    let message = element(&body, "Message").unwrap_or("");
    anyhow::bail!("{what}: {status} {code} {message}")
}

/// The text of the first `<name>…</name>` in `xml`. COS answers are flat and
/// small; a parser would be the heaviest thing in this module for this.
fn element<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&format!("</{name}>"))? + start;
    Some(&xml[start..end])
}

fn content_md5(body: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(Md5::digest(body))
}

/// The `Authorization` header COS expects, for a request valid from `start` to
/// `end` (unix seconds).
///
/// Only the parameters and headers passed here are signed. The query parameters
/// must be every one on the URL, and `host` is the one header worth binding: the
/// rest either change in transit or are already covered by `Content-MD5`.
#[allow(clippy::too_many_arguments)]
fn authorization(
    secret_id: &str,
    secret_key: &str,
    method: &str,
    path: &str,
    params: &[(&str, &str)],
    headers: &[(&str, &str)],
    start: i64,
    end: i64,
) -> String {
    let key_time = format!("{start};{end}");
    let sign_key = super::hex(&hmac_sha1(secret_key.as_bytes(), key_time.as_bytes()));

    let (param_list, http_params) = canonical(params);
    let (header_list, http_headers) = canonical(headers);
    let http_string = format!(
        "{}\n{path}\n{http_params}\n{http_headers}\n",
        method.to_ascii_lowercase()
    );
    let string_to_sign = format!(
        "sha1\n{key_time}\n{}\n",
        super::hex(&Sha1::digest(http_string.as_bytes()))
    );
    let signature = super::hex(&hmac_sha1(sign_key.as_bytes(), string_to_sign.as_bytes()));

    format!(
        "q-sign-algorithm=sha1&q-ak={secret_id}&q-sign-time={key_time}&q-key-time={key_time}\
         &q-header-list={header_list}&q-url-param-list={param_list}&q-signature={signature}"
    )
}

/// `(names, pairs)` for signing: keys lower-cased and encoded, sorted, names
/// joined by `;` and `name=value` pairs by `&`.
fn canonical(pairs: &[(&str, &str)]) -> (String, String) {
    let mut v: Vec<(String, String)> =
        pairs.iter().map(|(k, val)| (encode(&k.to_ascii_lowercase()), encode(val))).collect();
    v.sort();
    let names = v.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
    let joined = v.iter().map(|(k, val)| format!("{k}={val}")).collect::<Vec<_>>().join("&");
    (names, joined)
}

/// RFC 3986 percent-encoding: unreserved characters stay, everything else is
/// `%XX` in upper case — the encoding COS signs over.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn hmac_sha1(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha1>::new_from_slice(key).expect("HMAC takes a key of any length");
    mac.update(msg);
    mac.finalize().into_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_lists_are_sorted_lower_cased_and_encoded() {
        let (names, pairs) = canonical(&[("uploadId", "a b"), ("partNumber", "2")]);
        assert_eq!(names, "partnumber;uploadid");
        assert_eq!(pairs, "partnumber=2&uploadid=a%20b");
    }

    /// A parameter with no value is still signed, as `name=` — `?uploads` is the
    /// one this module sends.
    #[test]
    fn a_bare_parameter_signs_with_an_empty_value() {
        let (names, pairs) = canonical(&[("uploads", "")]);
        assert_eq!((names.as_str(), pairs.as_str()), ("uploads", "uploads="));
    }

    #[test]
    fn the_header_names_what_was_signed() {
        let a = authorization("AKID", "secret", "PUT", "/ana/x.jpg", &[], &[("host", "b.cos")], 1, 2);
        assert!(a.starts_with("q-sign-algorithm=sha1&q-ak=AKID&q-sign-time=1;2&q-key-time=1;2"));
        assert!(a.contains("&q-header-list=host&q-url-param-list=&q-signature="));
        // Deterministic: the same request signs the same way.
        assert_eq!(a, authorization("AKID", "secret", "PUT", "/ana/x.jpg", &[], &[("host", "b.cos")], 1, 2));
    }

    #[test]
    fn element_reads_a_flat_answer() {
        let xml = "<InitiateMultipartUploadResult><Bucket>b</Bucket><UploadId>123abc</UploadId></InitiateMultipartUploadResult>";
        assert_eq!(element(xml, "UploadId"), Some("123abc"));
        assert_eq!(element(xml, "Missing"), None);
    }
}
