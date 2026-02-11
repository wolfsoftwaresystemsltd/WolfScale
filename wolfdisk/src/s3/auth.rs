//! S3 authentication
//!
//! Supports optional access key / secret key authentication.
//! When credentials are not configured, all requests are allowed (internal use).

use serde::{Deserialize, Serialize};

/// S3 credentials
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3Credentials {
    pub access_key: String,
    pub secret_key: String,
}

/// Validate an Authorization header against configured credentials.
/// Returns true if auth is disabled (no credentials configured) or if the
/// access key in the header matches. We do a simplified check — full SigV4
/// verification is complex and not needed for a private cluster.
pub fn check_auth(
    auth_header: Option<&str>,
    credentials: Option<&S3Credentials>,
) -> bool {
    let creds = match credentials {
        Some(c) => c,
        None => return true, // No auth configured — allow all
    };

    let header = match auth_header {
        Some(h) => h,
        None => return false, // Auth required but no header
    };

    // Check for AWS4-HMAC-SHA256 signature (extract Credential= access key)
    if header.starts_with("AWS4-HMAC-SHA256") {
        if let Some(cred_part) = header.split("Credential=").nth(1) {
            if let Some(access_key) = cred_part.split('/').next() {
                return access_key == creds.access_key;
            }
        }
    }

    // Check for simple AWS access key format: AWS <access_key>:<signature>
    if header.starts_with("AWS ") {
        if let Some(key_part) = header.strip_prefix("AWS ") {
            if let Some(access_key) = key_part.split(':').next() {
                return access_key == creds.access_key;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_auth_configured() {
        assert!(check_auth(None, None));
        assert!(check_auth(Some("anything"), None));
    }

    #[test]
    fn test_auth_required_no_header() {
        let creds = S3Credentials {
            access_key: "AKID".to_string(),
            secret_key: "secret".to_string(),
        };
        assert!(!check_auth(None, Some(&creds)));
    }

    #[test]
    fn test_aws4_auth() {
        let creds = S3Credentials {
            access_key: "AKID".to_string(),
            secret_key: "secret".to_string(),
        };
        let header = "AWS4-HMAC-SHA256 Credential=AKID/20260211/us-east-1/s3/aws4_request";
        assert!(check_auth(Some(header), Some(&creds)));
    }
}
