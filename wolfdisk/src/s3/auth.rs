//! S3 authentication — AWS Signature Version 4 verification.
//!
//! When no credentials are configured the gateway allows all requests (the
//! historical private-cluster behaviour). When credentials *are* configured we
//! perform full SigV4 verification (header-based and presigned query), instead
//! of merely string-matching the access key.
//!
//! The canonical URI is URI-encoded once (the Amazon S3 rule). The payload hash
//! used in the canonical request is the value the client put in
//! `x-amz-content-sha256` verbatim, which makes verification independent of the
//! body and supports `UNSIGNED-PAYLOAD` and `STREAMING-AWS4-HMAC-SHA256-PAYLOAD`.

use axum::http::{HeaderMap, StatusCode};
use hmac::{Hmac, Mac};
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Maximum allowed clock skew between client and server (header auth).
const MAX_SKEW_SECS: i64 = 15 * 60;

/// S3 credentials
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3Credentials {
    pub access_key: String,
    pub secret_key: String,
}

/// An authentication failure, carrying the S3 error code to report.
#[derive(Debug)]
pub struct AuthError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl AuthError {
    fn forbidden(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code,
            message: message.into(),
        }
    }
    fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message: message.into(),
        }
    }
}

/// Verify a request. Returns `Ok(())` when the request is authorised — including
/// the case where no credentials are configured (auth disabled).
pub fn verify(
    method: &str,
    raw_path: &str,
    raw_query: &str,
    headers: &HeaderMap,
    credentials: Option<&S3Credentials>,
) -> Result<(), AuthError> {
    let creds = match credentials {
        // No credentials configured → auth disabled, allow all.
        None => return Ok(()),
        Some(c) => c,
    };

    if raw_query.contains("X-Amz-Signature=") {
        verify_presigned(method, raw_path, raw_query, headers, creds)
    } else {
        verify_header(method, raw_path, raw_query, headers, creds)
    }
}

fn verify_header(
    method: &str,
    raw_path: &str,
    raw_query: &str,
    headers: &HeaderMap,
    creds: &S3Credentials,
) -> Result<(), AuthError> {
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AuthError::forbidden("AccessDenied", "Missing Authorization header"))?;

    let rest = auth
        .strip_prefix("AWS4-HMAC-SHA256 ")
        .ok_or_else(|| AuthError::forbidden("AccessDenied", "Unsupported authorization scheme"))?;

    let mut credential = None;
    let mut signed_headers_str = None;
    let mut provided_sig = None;
    for part in rest.split(',') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix("Credential=") {
            credential = Some(v.to_string());
        } else if let Some(v) = part.strip_prefix("SignedHeaders=") {
            signed_headers_str = Some(v.to_string());
        } else if let Some(v) = part.strip_prefix("Signature=") {
            provided_sig = Some(v.to_string());
        }
    }
    let credential = credential
        .ok_or_else(|| AuthError::bad_request("InvalidArgument", "Missing Credential"))?;
    let signed_headers_str = signed_headers_str
        .ok_or_else(|| AuthError::bad_request("InvalidArgument", "Missing SignedHeaders"))?;
    let provided_sig = provided_sig
        .ok_or_else(|| AuthError::bad_request("InvalidArgument", "Missing Signature"))?;

    let scope = parse_credential(&credential)?;
    if scope.access_key != creds.access_key {
        return Err(AuthError::forbidden(
            "InvalidAccessKeyId",
            "The access key Id you provided does not exist in our records.",
        ));
    }

    let amz_date = header_value(headers, "x-amz-date")
        .ok_or_else(|| AuthError::bad_request("InvalidArgument", "Missing x-amz-date"))?;
    check_skew(&amz_date)?;

    let payload_hash =
        header_value(headers, "x-amz-content-sha256").unwrap_or_else(|| "UNSIGNED-PAYLOAD".into());

    let signed_headers: Vec<String> = signed_headers_str
        .split(';')
        .map(|s| s.to_lowercase())
        .collect();
    let canon_headers = canonical_headers(headers, &signed_headers)?;
    let canon_uri = canonical_uri(raw_path);
    let canon_query = canonical_query(raw_query, &[]);
    let canon_hash = canonical_request_hash(
        method,
        &canon_uri,
        &canon_query,
        &canon_headers,
        &signed_headers_str,
        &payload_hash,
    );

    let credential_scope = format!(
        "{}/{}/{}/aws4_request",
        scope.date_stamp, scope.region, scope.service
    );
    let sts = string_to_sign(&amz_date, &credential_scope, &canon_hash);
    let expected = compute_signature(
        &creds.secret_key,
        &scope.date_stamp,
        &scope.region,
        &scope.service,
        &sts,
    );

    if !ct_eq(expected.as_bytes(), provided_sig.as_bytes()) {
        return Err(AuthError::forbidden(
            "SignatureDoesNotMatch",
            "The request signature we calculated does not match the signature you provided.",
        ));
    }
    Ok(())
}

fn verify_presigned(
    method: &str,
    raw_path: &str,
    raw_query: &str,
    headers: &HeaderMap,
    creds: &S3Credentials,
) -> Result<(), AuthError> {
    let mut params: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for kv in raw_query.split('&') {
        if let Some((k, v)) = kv.split_once('=') {
            let k = percent_decode_str(k).decode_utf8_lossy().to_string();
            let v = percent_decode_str(v).decode_utf8_lossy().to_string();
            params.insert(k, v);
        }
    }
    let get = |name: &str| -> Result<String, AuthError> {
        params
            .get(name)
            .cloned()
            .ok_or_else(|| AuthError::bad_request("InvalidArgument", format!("Missing {}", name)))
    };

    if get("X-Amz-Algorithm")? != "AWS4-HMAC-SHA256" {
        return Err(AuthError::bad_request(
            "InvalidArgument",
            "Unsupported X-Amz-Algorithm",
        ));
    }
    let credential = get("X-Amz-Credential")?;
    let amz_date = get("X-Amz-Date")?;
    let signed_headers_str = get("X-Amz-SignedHeaders")?;
    let provided_sig = get("X-Amz-Signature")?;
    let expires: i64 = get("X-Amz-Expires")?
        .parse()
        .map_err(|_| AuthError::bad_request("InvalidArgument", "Invalid X-Amz-Expires"))?;

    {
        use chrono::{Duration, NaiveDateTime, Utc};
        let signed = NaiveDateTime::parse_from_str(&amz_date, "%Y%m%dT%H%M%SZ")
            .map_err(|_| AuthError::bad_request("InvalidArgument", "Invalid X-Amz-Date"))?;
        let now = Utc::now().naive_utc();
        if now > signed + Duration::seconds(expires) {
            return Err(AuthError::forbidden("AccessDenied", "Request has expired."));
        }
        if (now - signed).num_seconds() < -MAX_SKEW_SECS {
            return Err(AuthError::forbidden(
                "RequestTimeTooSkewed",
                "Request time is too skewed.",
            ));
        }
    }

    let scope = parse_credential(&credential)?;
    if scope.access_key != creds.access_key {
        return Err(AuthError::forbidden(
            "InvalidAccessKeyId",
            "The access key Id you provided does not exist in our records.",
        ));
    }

    let signed_headers: Vec<String> = signed_headers_str
        .split(';')
        .map(|s| s.to_lowercase())
        .collect();
    let canon_headers = canonical_headers(headers, &signed_headers)?;
    let canon_uri = canonical_uri(raw_path);
    let canon_query = canonical_query(raw_query, &["X-Amz-Signature"]);
    let canon_hash = canonical_request_hash(
        method,
        &canon_uri,
        &canon_query,
        &canon_headers,
        &signed_headers_str,
        "UNSIGNED-PAYLOAD",
    );

    let credential_scope = format!(
        "{}/{}/{}/aws4_request",
        scope.date_stamp, scope.region, scope.service
    );
    let sts = string_to_sign(&amz_date, &credential_scope, &canon_hash);
    let expected = compute_signature(
        &creds.secret_key,
        &scope.date_stamp,
        &scope.region,
        &scope.service,
        &sts,
    );
    if !ct_eq(expected.as_bytes(), provided_sig.as_bytes()) {
        return Err(AuthError::forbidden(
            "SignatureDoesNotMatch",
            "The request signature we calculated does not match the signature you provided.",
        ));
    }
    Ok(())
}

// ─── SigV4 primitives ────────────────────────────────────────────────────────

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn signing_key(secret: &str, date_stamp: &str, region: &str, service: &str) -> Vec<u8> {
    let k_date = hmac_sha256(format!("AWS4{}", secret).as_bytes(), date_stamp.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

/// URI-encode per RFC 3986 / SigV4. Slashes preserved when `keep_slash`.
fn uri_encode(input: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(input.len());
    for &b in input.as_bytes() {
        let unreserved =
            b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~';
        if unreserved || (keep_slash && b == b'/') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{:02X}", b));
        }
    }
    out
}

fn canonical_uri(raw_path: &str) -> String {
    let decoded = percent_decode_str(raw_path).decode_utf8_lossy();
    let encoded = uri_encode(&decoded, true);
    if encoded.is_empty() {
        "/".to_string()
    } else {
        encoded
    }
}

fn canonical_query(raw_query: &str, exclude: &[&str]) -> String {
    let mut pairs: Vec<(String, String)> = Vec::new();
    if !raw_query.is_empty() {
        for kv in raw_query.split('&') {
            if kv.is_empty() {
                continue;
            }
            let (k, v) = match kv.split_once('=') {
                Some((k, v)) => (k, v),
                None => (kv, ""),
            };
            let k_dec = percent_decode_str(k).decode_utf8_lossy().to_string();
            if exclude.iter().any(|e| e.eq_ignore_ascii_case(&k_dec)) {
                continue;
            }
            let v_dec = percent_decode_str(v).decode_utf8_lossy().to_string();
            pairs.push((uri_encode(&k_dec, false), uri_encode(&v_dec, false)));
        }
    }
    pairs.sort();
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&")
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let mut values: Vec<String> = Vec::new();
    for v in headers.get_all(name) {
        if let Ok(s) = v.to_str() {
            values.push(collapse_ws(s));
        }
    }
    if values.is_empty() {
        None
    } else {
        Some(values.join(","))
    }
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.trim().chars() {
        if ch == ' ' || ch == '\t' {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out
}

fn canonical_headers(headers: &HeaderMap, signed_headers: &[String]) -> Result<String, AuthError> {
    let mut block = String::new();
    for name in signed_headers {
        let value = header_value(headers, name).ok_or_else(|| {
            AuthError::bad_request(
                "InvalidArgument",
                format!("Signed header '{}' not present", name),
            )
        })?;
        block.push_str(name);
        block.push(':');
        block.push_str(&value);
        block.push('\n');
    }
    Ok(block)
}

fn canonical_request_hash(
    method: &str,
    canon_uri: &str,
    canon_query: &str,
    canon_headers: &str,
    signed_headers_list: &str,
    payload_hash: &str,
) -> String {
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method, canon_uri, canon_query, canon_headers, signed_headers_list, payload_hash
    );
    sha256_hex(canonical_request.as_bytes())
}

fn string_to_sign(amz_date: &str, scope: &str, canon_req_hash: &str) -> String {
    format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        amz_date, scope, canon_req_hash
    )
}

fn compute_signature(
    secret: &str,
    date_stamp: &str,
    region: &str,
    service: &str,
    sts: &str,
) -> String {
    let key = signing_key(secret, date_stamp, region, service);
    hex::encode(hmac_sha256(&key, sts.as_bytes()))
}

/// Constant-time byte comparison (avoids signature timing oracles).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

struct CredentialScope {
    access_key: String,
    date_stamp: String,
    region: String,
    service: String,
}

fn parse_credential(cred: &str) -> Result<CredentialScope, AuthError> {
    let parts: Vec<&str> = cred.split('/').collect();
    if parts.len() != 5 || parts[4] != "aws4_request" {
        return Err(AuthError::bad_request(
            "InvalidArgument",
            "Malformed Credential scope",
        ));
    }
    Ok(CredentialScope {
        access_key: parts[0].to_string(),
        date_stamp: parts[1].to_string(),
        region: parts[2].to_string(),
        service: parts[3].to_string(),
    })
}

fn check_skew(amz_date: &str) -> Result<(), AuthError> {
    use chrono::{NaiveDateTime, Utc};
    let parsed = NaiveDateTime::parse_from_str(amz_date, "%Y%m%dT%H%M%SZ")
        .map_err(|_| AuthError::bad_request("InvalidArgument", "Invalid x-amz-date"))?;
    let now = Utc::now().naive_utc();
    if (now - parsed).num_seconds().abs() > MAX_SKEW_SECS {
        return Err(AuthError::forbidden(
            "RequestTimeTooSkewed",
            "The difference between the request time and the server's time is too large.",
        ));
    }
    Ok(())
}

/// Decode an `aws-chunked` (STREAMING-AWS4-HMAC-SHA256-PAYLOAD) body into the
/// underlying object bytes. Per-chunk signatures are not re-verified (the
/// request is authenticated by its seed signature); only the framing is
/// stripped. Wire format per chunk:
/// `<hex-size>;chunk-signature=<sig>\r\n<payload>\r\n`, ending with a 0-size chunk.
pub fn decode_aws_chunked(body: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(body.len());
    let mut pos = 0usize;
    loop {
        let line_end = find_crlf(body, pos).ok_or("malformed aws-chunked body")?;
        let line = &body[pos..line_end];
        let size_str = match line.iter().position(|&b| b == b';') {
            Some(idx) => &line[..idx],
            None => line,
        };
        let size_text = std::str::from_utf8(size_str)
            .map_err(|_| "invalid chunk size")?
            .trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| "invalid chunk size")?;
        pos = line_end + 2;
        if size == 0 {
            break;
        }
        if pos + size > body.len() {
            return Err("truncated aws-chunked body".into());
        }
        out.extend_from_slice(&body[pos..pos + size]);
        pos += size;
        if body.get(pos) == Some(&b'\r') && body.get(pos + 1) == Some(&b'\n') {
            pos += 2;
        } else {
            return Err("missing chunk terminator".into());
        }
    }
    Ok(out)
}

fn find_crlf(buf: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 1 < buf.len() {
        if buf[i] == b'\r' && buf[i + 1] == b'\n' {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // AWS-published S3 GET example. Ground-truth signature computed
    // independently with a reference HMAC implementation.
    #[test]
    fn aws_published_get_object_vector() {
        let secret = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
        let canonical_request = "GET\n/test.txt\n\nhost:examplebucket.s3.amazonaws.com\nrange:bytes=0-9\nx-amz-content-sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\nx-amz-date:20130524T000000Z\n\nhost;range;x-amz-content-sha256;x-amz-date\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let canon_hash = sha256_hex(canonical_request.as_bytes());
        assert_eq!(
            canon_hash,
            "7344ae5b7ee6c3e7e6b0fe0640412a37625d1fbfff95c48bbb2dc43964946972"
        );
        let scope = "20130524/us-east-1/s3/aws4_request";
        let sts = string_to_sign("20130524T000000Z", scope, &canon_hash);
        let sig = compute_signature(secret, "20130524", "us-east-1", "s3", &sts);
        assert_eq!(
            sig,
            "67fe34c8530db585abddc51067328adfedb6e42487d2566dc7d927d6e2722900"
        );
    }

    #[test]
    fn no_credentials_allows_all() {
        let headers = HeaderMap::new();
        assert!(verify("GET", "/", "", &headers, None).is_ok());
    }

    #[test]
    fn missing_auth_denied_when_creds_set() {
        let creds = S3Credentials {
            access_key: "AKID".into(),
            secret_key: "secret".into(),
        };
        let headers = HeaderMap::new();
        assert!(verify("GET", "/", "", &headers, Some(&creds)).is_err());
    }

    #[test]
    fn uri_encode_rules() {
        assert_eq!(uri_encode("/a b/c.txt", true), "/a%20b/c.txt");
        assert_eq!(uri_encode("a/b", false), "a%2Fb");
    }

    #[test]
    fn aws_chunked_roundtrip() {
        let body = b"5;chunk-signature=x\r\nhello\r\n6;chunk-signature=y\r\n world\r\n0;chunk-signature=z\r\n\r\n";
        assert_eq!(decode_aws_chunked(body).unwrap(), b"hello world");
    }

    #[test]
    fn ct_eq_works() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }
}
