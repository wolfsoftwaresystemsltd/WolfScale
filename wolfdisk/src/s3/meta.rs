//! S3-specific object metadata sidecar + multipart upload registry.
//!
//! These live entirely within the S3 module so the shared `FileEntry`/index/
//! replication types are left untouched. Object content, size and chunks are
//! stored in the normal WolfDisk index (and replicate as usual); only the
//! S3-flavoured extras — Content-Type, the S3 ETag, and `x-amz-meta-*` user
//! metadata — are kept here, keyed by `"bucket/key"`.
//!
//! Consequence (documented): S3 metadata is node-local — it is not carried by
//! the replication wire protocol, exactly like the existing `symlink_target`
//! field. Objects still replicate; their S3 metadata is reconstructed with
//! defaults on other nodes.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::storage::ChunkRef;

/// S3 metadata for a single object.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct S3ObjectMeta {
    #[serde(default)]
    pub content_type: Option<String>,
    /// S3 ETag without surrounding quotes (hex MD5, or `<hex>-<n>` for multipart).
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub user_meta: HashMap<String, String>,
}

/// Persistent sidecar map of `"bucket/key"` → metadata.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct S3MetaStore {
    entries: HashMap<String, S3ObjectMeta>,
}

impl S3MetaStore {
    /// Load the sidecar from `path`, or start empty if it doesn't exist / can't
    /// be parsed (best-effort; never fatal).
    pub fn load(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Persist atomically (temp file + rename). Best-effort.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        let data = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, &data)?;
        std::fs::rename(&tmp, path)
    }

    pub fn get(&self, key: &str) -> Option<&S3ObjectMeta> {
        self.entries.get(key)
    }

    pub fn put(&mut self, key: String, meta: S3ObjectMeta) {
        self.entries.insert(key, meta);
    }

    pub fn remove(&mut self, key: &str) {
        self.entries.remove(key);
    }
}

/// In-progress multipart upload (in-memory only; aborted on restart).
#[derive(Debug, Clone)]
pub struct MultipartUpload {
    pub bucket: String,
    pub key: String,
    pub content_type: Option<String>,
    pub user_meta: HashMap<String, String>,
    /// part number → uploaded part
    pub parts: BTreeMap<u32, PartInfo>,
}

#[derive(Debug, Clone)]
pub struct PartInfo {
    /// Chunk refs with offsets relative to the start of the part.
    pub chunks: Vec<ChunkRef>,
    /// Hex MD5 of the part data (the part ETag).
    pub md5_hex: String,
    pub size: u64,
}

/// Build the sidecar file path for a data directory's index dir.
pub fn meta_path(index_dir: &Path) -> PathBuf {
    index_dir.join("s3_meta.json")
}

// ─── Tolerant XML parsing for request bodies ─────────────────────────────────

fn unescape_xml(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// Extract the text content of the first `<tag>...</tag>` within `block`.
fn extract_tag(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let s = block.find(&open)? + open.len();
    let e = block[s..].find(&close)? + s;
    Some(unescape_xml(&block[s..e]))
}

/// Parse a CompleteMultipartUpload body into ordered `(part_number, etag)` pairs.
pub fn parse_complete_multipart(body: &[u8]) -> Result<Vec<(u32, String)>, String> {
    let s = std::str::from_utf8(body).map_err(|_| "invalid UTF-8".to_string())?;
    let mut parts = Vec::new();
    let mut idx = 0;
    while let Some(rel) = s[idx..].find("<Part>") {
        let start = idx + rel;
        let end_rel = s[start..].find("</Part>").ok_or("unterminated <Part>")?;
        let end = start + end_rel;
        let block = &s[start..end];
        let num = extract_tag(block, "PartNumber").ok_or("missing PartNumber")?;
        let etag = extract_tag(block, "ETag").ok_or("missing ETag")?;
        let num: u32 = num.trim().parse().map_err(|_| "invalid PartNumber")?;
        parts.push((num, etag.trim().trim_matches('"').to_string()));
        idx = end + "</Part>".len();
    }
    if parts.is_empty() {
        return Err("no parts in CompleteMultipartUpload".to_string());
    }
    Ok(parts)
}

/// Parse a DeleteObjects body into `(keys, quiet)`.
pub fn parse_delete(body: &[u8]) -> Result<(Vec<String>, bool), String> {
    let s = std::str::from_utf8(body).map_err(|_| "invalid UTF-8".to_string())?;
    let quiet = extract_tag(s, "Quiet")
        .map(|v| v.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let mut keys = Vec::new();
    let mut idx = 0;
    while let Some(rel) = s[idx..].find("<Object>") {
        let start = idx + rel;
        let end_rel = s[start..]
            .find("</Object>")
            .ok_or("unterminated <Object>")?;
        let end = start + end_rel;
        let block = &s[start..end];
        if let Some(key) = extract_tag(block, "Key") {
            keys.push(key);
        }
        idx = end + "</Object>".len();
    }
    Ok((keys, quiet))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_complete_multipart_ok() {
        let xml = r#"<CompleteMultipartUpload>
            <Part><PartNumber>1</PartNumber><ETag>"abc"</ETag></Part>
            <Part><PartNumber>2</PartNumber><ETag>def</ETag></Part>
        </CompleteMultipartUpload>"#;
        let parts = parse_complete_multipart(xml.as_bytes()).unwrap();
        assert_eq!(parts, vec![(1, "abc".into()), (2, "def".into())]);
    }

    #[test]
    fn parse_delete_ok() {
        let xml = r#"<Delete><Quiet>true</Quiet>
            <Object><Key>a/b.txt</Key></Object>
            <Object><Key>c.txt</Key></Object></Delete>"#;
        let (keys, quiet) = parse_delete(xml.as_bytes()).unwrap();
        assert!(quiet);
        assert_eq!(keys, vec!["a/b.txt".to_string(), "c.txt".to_string()]);
    }

    #[test]
    fn unescape_in_key() {
        let xml = "<Delete><Object><Key>a&amp;b.txt</Key></Object></Delete>";
        let (keys, _) = parse_delete(xml.as_bytes()).unwrap();
        assert_eq!(keys, vec!["a&b.txt".to_string()]);
    }
}
