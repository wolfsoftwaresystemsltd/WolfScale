//! S3-compatible HTTP server for WolfDisk
//!
//! Maps WolfDisk's file index and chunk store to S3 buckets and objects:
//! - Top-level directories → S3 buckets
//! - Files within directories → S3 objects
//! - Files at root level → objects in a virtual "default" bucket
//!
//! Supports: ListBuckets, ListObjectsV2, GetObject, PutObject, DeleteObject,
//! HeadObject, HeadBucket, CreateBucket, DeleteBucket

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, Method, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use tokio::net::TcpListener;
use tracing::{info, error, debug};

use crate::storage::{ChunkStore, FileIndex, FileEntry, ChunkRef, InodeTable};
use super::auth::{S3Credentials, check_auth};

/// Shared state for the S3 server
#[derive(Clone)]
pub struct S3State {
    pub file_index: Arc<RwLock<FileIndex>>,
    pub chunk_store: Arc<ChunkStore>,
    pub inode_table: Arc<RwLock<InodeTable>>,
    pub next_inode: Arc<RwLock<u64>>,
    pub credentials: Option<S3Credentials>,
    pub region: String,
}

/// S3 server that runs alongside WolfDisk FUSE
pub struct S3Server {
    bind_addr: String,
    state: S3State,
}

impl S3Server {
    /// Create a new S3 server
    pub fn new(
        bind_addr: String,
        file_index: Arc<RwLock<FileIndex>>,
        chunk_store: Arc<ChunkStore>,
        inode_table: Arc<RwLock<InodeTable>>,
        next_inode: Arc<RwLock<u64>>,
        credentials: Option<S3Credentials>,
    ) -> Self {
        let state = S3State {
            file_index,
            chunk_store,
            inode_table,
            next_inode,
            credentials,
            region: "us-east-1".to_string(),
        };

        Self { bind_addr, state }
    }

    /// Start the S3 server (call from a tokio runtime)
    pub async fn run(self) -> std::io::Result<()> {
        let app = Router::new()
            // Catch-all route — S3 routing is path-based
            .route("/", any(handle_root))
            .route("/{*path}", any(handle_path))
            .with_state(self.state.clone());

        info!("S3-compatible API listening on {}", self.bind_addr);

        let listener = TcpListener::bind(&self.bind_addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }
}

// ─── Root handler (ListBuckets) ──────────────────────────────────────────────

async fn handle_root(
    State(state): State<S3State>,
    headers: HeaderMap,
    method: Method,
) -> Response {
    if !authorize(&headers, &state.credentials) {
        return error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
    }

    match method {
        Method::GET => list_buckets(state).await,
        _ => error_response(StatusCode::METHOD_NOT_ALLOWED, "MethodNotAllowed", "Method not allowed"),
    }
}

// ─── Path handler (dispatches to bucket or object operations) ────────────────

async fn handle_path(
    State(state): State<S3State>,
    Path(path): Path<String>,
    headers: HeaderMap,
    method: Method,
    query: Query<HashMap<String, String>>,
    request: Request<Body>,
) -> Response {
    if !authorize(&headers, &state.credentials) {
        return error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
    }

    // Parse bucket and key from path
    let (bucket, key) = parse_bucket_key(&path);

    match (method, key) {
        // ── Bucket-level operations ────────────────────────────
        (Method::GET, None) => {
            // Could be ListObjectsV2 or GetBucketLocation
            if query.contains_key("location") {
                get_bucket_location(state).await
            } else {
                list_objects(state, &bucket, &query).await
            }
        }
        (Method::HEAD, None) => head_bucket(state, &bucket).await,
        (Method::PUT, None) => create_bucket(state, &bucket).await,
        (Method::DELETE, None) => delete_bucket(state, &bucket).await,

        // ── Object-level operations ────────────────────────────
        (Method::GET, Some(key)) => get_object(state, &bucket, &key).await,
        (Method::HEAD, Some(key)) => head_object(state, &bucket, &key).await,
        (Method::PUT, Some(key)) => {
            // Read the body
            let body_bytes = match axum::body::to_bytes(request.into_body(), 512 * 1024 * 1024).await {
                Ok(b) => b,
                Err(e) => {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        "InvalidRequest",
                        &format!("Failed to read body: {}", e),
                    );
                }
            };
            put_object(state, &bucket, &key, body_bytes.to_vec()).await
        }
        (Method::DELETE, Some(key)) => delete_object(state, &bucket, &key).await,

        _ => error_response(StatusCode::METHOD_NOT_ALLOWED, "MethodNotAllowed", "Method not allowed"),
    }
}

// ─── S3 Operations ───────────────────────────────────────────────────────────

/// GET / → ListBuckets
async fn list_buckets(state: S3State) -> Response {
    let index = state.file_index.read().unwrap();

    // Collect unique top-level directories as buckets
    let mut buckets: HashSet<String> = HashSet::new();
    for (path, entry) in index.iter() {
        if entry.is_dir {
            // Top-level dirs become buckets
            let components: Vec<_> = path.components().collect();
            if components.len() == 1 {
                if let Some(name) = path.file_name() {
                    buckets.insert(name.to_string_lossy().to_string());
                }
            }
        } else {
            // Files at root level — infer bucket from first component
            if let Some(first) = path.components().next() {
                let first_str = first.as_os_str().to_string_lossy().to_string();
                // Only count as a bucket if there are more components (i.e., it's inside a dir)
                if path.components().count() > 1 {
                    buckets.insert(first_str);
                }
            }
        }
    }

    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<ListAllMyBucketsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\n");
    xml.push_str("  <Owner>\n");
    xml.push_str("    <ID>wolfdisk</ID>\n");
    xml.push_str("    <DisplayName>WolfDisk</DisplayName>\n");
    xml.push_str("  </Owner>\n");
    xml.push_str("  <Buckets>\n");

    for bucket_name in &buckets {
        xml.push_str("    <Bucket>\n");
        xml.push_str(&format!("      <Name>{}</Name>\n", xml_escape(bucket_name)));
        xml.push_str("      <CreationDate>2025-01-01T00:00:00.000Z</CreationDate>\n");
        xml.push_str("    </Bucket>\n");
    }

    xml.push_str("  </Buckets>\n");
    xml.push_str("</ListAllMyBucketsResult>");

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/xml")],
        xml,
    ).into_response()
}

/// GET /bucket?location → GetBucketLocation
async fn get_bucket_location(state: S3State) -> Response {
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <LocationConstraint xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">{}</LocationConstraint>",
        state.region
    );

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/xml")],
        xml,
    ).into_response()
}

/// GET /bucket → ListObjectsV2
async fn list_objects(
    state: S3State,
    bucket: &str,
    query: &HashMap<String, String>,
) -> Response {
    let prefix = query.get("prefix").cloned().unwrap_or_default();
    let delimiter = query.get("delimiter").cloned().unwrap_or_default();
    let max_keys: usize = query
        .get("max-keys")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);
    let continuation_token = query.get("continuation-token").cloned();

    let index = state.file_index.read().unwrap();
    let bucket_prefix = PathBuf::from(bucket);

    let mut objects: Vec<(String, &FileEntry)> = Vec::new();
    let mut common_prefixes: HashSet<String> = HashSet::new();

    for (path, entry) in index.iter() {
        // Only include files under this bucket
        if !path.starts_with(&bucket_prefix) {
            continue;
        }

        // Get the key (path relative to the bucket)
        let key = path
            .strip_prefix(&bucket_prefix)
            .unwrap()
            .to_string_lossy()
            .to_string();

        // Skip the bucket directory itself
        if key.is_empty() {
            continue;
        }

        // Skip directory entries themselves (they show up as common prefixes)
        if entry.is_dir {
            // If there is a delimiter, add as common prefix
            if !delimiter.is_empty() {
                let dir_prefix = format!("{}/", key.trim_end_matches('/'));
                if dir_prefix.starts_with(&prefix) {
                    common_prefixes.insert(dir_prefix);
                }
            }
            continue;
        }

        // Apply prefix filter
        if !key.starts_with(&prefix) {
            continue;
        }

        // Handle delimiter (directory grouping)
        if !delimiter.is_empty() {
            let after_prefix = &key[prefix.len()..];
            if let Some(delim_pos) = after_prefix.find(&delimiter) {
                let common = format!("{}{}{}", prefix, &after_prefix[..delim_pos], delimiter);
                common_prefixes.insert(common);
                continue;
            }
        }

        objects.push((key, entry));
    }

    // Sort by key
    objects.sort_by(|a, b| a.0.cmp(&b.0));

    // Apply continuation token (simple: skip until we find the token key)
    if let Some(ref token) = continuation_token {
        if let Some(pos) = objects.iter().position(|(k, _)| k.as_str() > token.as_str()) {
            objects = objects.split_off(pos);
        } else {
            objects.clear();
        }
    }

    let is_truncated = objects.len() > max_keys;
    let objects: Vec<_> = objects.into_iter().take(max_keys).collect();

    let next_token = if is_truncated {
        objects.last().map(|(k, _)| k.clone())
    } else {
        None
    };

    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\n");
    xml.push_str(&format!("  <Name>{}</Name>\n", xml_escape(bucket)));
    xml.push_str(&format!("  <Prefix>{}</Prefix>\n", xml_escape(&prefix)));
    xml.push_str(&format!("  <MaxKeys>{}</MaxKeys>\n", max_keys));
    xml.push_str(&format!("  <IsTruncated>{}</IsTruncated>\n", is_truncated));
    xml.push_str("  <KeyCount>");
    xml.push_str(&objects.len().to_string());
    xml.push_str("</KeyCount>\n");

    if let Some(ref token) = next_token {
        xml.push_str(&format!("  <NextContinuationToken>{}</NextContinuationToken>\n", xml_escape(token)));
    }

    for (key, entry) in &objects {
        xml.push_str("  <Contents>\n");
        xml.push_str(&format!("    <Key>{}</Key>\n", xml_escape(key)));
        xml.push_str(&format!("    <Size>{}</Size>\n", entry.size));
        xml.push_str(&format!("    <LastModified>{}</LastModified>\n", format_time(&entry.modified)));
        xml.push_str("    <StorageClass>STANDARD</StorageClass>\n");

        // ETag: use hash of first chunk if available, else empty
        let etag = if let Some(chunk) = entry.chunks.first() {
            format!("\"{}\"", hex::encode(&chunk.hash[..16]))
        } else {
            "\"d41d8cd98f00b204e9800998ecf8427e\"".to_string() // MD5 of empty
        };
        xml.push_str(&format!("    <ETag>{}</ETag>\n", etag));

        xml.push_str("  </Contents>\n");
    }

    // Common prefixes
    let mut sorted_prefixes: Vec<_> = common_prefixes.into_iter().collect();
    sorted_prefixes.sort();
    for cp in &sorted_prefixes {
        xml.push_str("  <CommonPrefixes>\n");
        xml.push_str(&format!("    <Prefix>{}</Prefix>\n", xml_escape(cp)));
        xml.push_str("  </CommonPrefixes>\n");
    }

    xml.push_str("</ListBucketResult>");

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/xml")],
        xml,
    ).into_response()
}

/// HEAD /bucket → HeadBucket
async fn head_bucket(state: S3State, bucket: &str) -> Response {
    let index = state.file_index.read().unwrap();
    let bucket_path = PathBuf::from(bucket);

    // Check if bucket exists (as a directory or has any files underneath)
    let exists = index.get(&bucket_path).map(|e| e.is_dir).unwrap_or(false)
        || index.iter().any(|(p, _)| p.starts_with(&bucket_path));

    if exists {
        (StatusCode::OK, [(header::CONTENT_TYPE, "application/xml")]).into_response()
    } else {
        error_response(StatusCode::NOT_FOUND, "NoSuchBucket", "The specified bucket does not exist")
    }
}

/// PUT /bucket → CreateBucket
async fn create_bucket(state: S3State, bucket: &str) -> Response {
    let bucket_path = PathBuf::from(bucket);

    {
        let mut index = state.file_index.write().unwrap();
        let mut inode_tbl = state.inode_table.write().unwrap();

        if index.contains(&bucket_path) {
            return error_response(
                StatusCode::CONFLICT,
                "BucketAlreadyExists",
                "The requested bucket already exists",
            );
        }

        let now = SystemTime::now();
        let entry = FileEntry {
            size: 0,
            is_dir: true,
            permissions: 0o755,
            uid: 0,
            gid: 0,
            created: now,
            modified: now,
            accessed: now,
            chunks: Vec::new(),
            symlink_target: None,
        };

        index.insert(bucket_path.clone(), entry);

        // Allocate inode
        let mut next_ino = state.next_inode.write().unwrap();
        let ino = *next_ino;
        *next_ino += 1;
        inode_tbl.insert(ino, bucket_path);
    }

    info!("S3: Created bucket '{}'", bucket);
    (StatusCode::OK, [(header::CONTENT_TYPE, "application/xml")]).into_response()
}

/// DELETE /bucket → DeleteBucket
async fn delete_bucket(state: S3State, bucket: &str) -> Response {
    let bucket_path = PathBuf::from(bucket);

    {
        let mut index = state.file_index.write().unwrap();
        let mut inode_tbl = state.inode_table.write().unwrap();

        // Check bucket exists
        match index.get(&bucket_path) {
            Some(e) if !e.is_dir => {
                return error_response(StatusCode::NOT_FOUND, "NoSuchBucket", "Not a bucket");
            }
            None => {
                return error_response(StatusCode::NOT_FOUND, "NoSuchBucket", "Bucket not found");
            }
            _ => {}
        }

        // Check bucket is empty
        let has_children = index.iter().any(|(p, _)| {
            p.starts_with(&bucket_path) && p != &bucket_path
        });

        if has_children {
            return error_response(
                StatusCode::CONFLICT,
                "BucketNotEmpty",
                "The bucket is not empty",
            );
        }

        index.remove(&bucket_path);
        inode_tbl.remove_path(&bucket_path);
    }

    info!("S3: Deleted bucket '{}'", bucket);
    (StatusCode::NO_CONTENT, [(header::CONTENT_TYPE, "application/xml")]).into_response()
}

/// GET /bucket/key → GetObject
async fn get_object(state: S3State, bucket: &str, key: &str) -> Response {
    let object_path = PathBuf::from(bucket).join(key);

    let index = state.file_index.read().unwrap();
    let entry = match index.get(&object_path) {
        Some(e) if !e.is_dir => e.clone(),
        Some(_) => {
            return error_response(StatusCode::NOT_FOUND, "NoSuchKey", "Key is a directory");
        }
        None => {
            return error_response(StatusCode::NOT_FOUND, "NoSuchKey", "The specified key does not exist");
        }
    };
    drop(index);

    // Read all chunk data
    let data = match state.chunk_store.read(&entry.chunks, 0, entry.size as usize) {
        Ok(d) => d,
        Err(e) => {
            error!("S3 GetObject: failed to read chunks for {}/{}: {}", bucket, key, e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "InternalError", "Failed to read object data");
        }
    };

    let etag = if let Some(chunk) = entry.chunks.first() {
        format!("\"{}\"", hex::encode(&chunk.hash[..16]))
    } else {
        "\"d41d8cd98f00b204e9800998ecf8427e\"".to_string()
    };

    debug!("S3 GetObject: {}/{} ({} bytes)", bucket, key, data.len());

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, data.len().to_string())
        .header("ETag", etag)
        .header("Last-Modified", format_time_http(&entry.modified))
        .body(Body::from(data))
        .unwrap()
}

/// HEAD /bucket/key → HeadObject
async fn head_object(state: S3State, bucket: &str, key: &str) -> Response {
    let object_path = PathBuf::from(bucket).join(key);

    let index = state.file_index.read().unwrap();
    match index.get(&object_path) {
        Some(entry) if !entry.is_dir => {
            let etag = if let Some(chunk) = entry.chunks.first() {
                format!("\"{}\"", hex::encode(&chunk.hash[..16]))
            } else {
                "\"d41d8cd98f00b204e9800998ecf8427e\"".to_string()
            };

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .header(header::CONTENT_LENGTH, entry.size.to_string())
                .header("ETag", etag)
                .header("Last-Modified", format_time_http(&entry.modified))
                .body(Body::empty())
                .unwrap()
        }
        _ => error_response(StatusCode::NOT_FOUND, "NoSuchKey", "Key not found"),
    }
}

/// PUT /bucket/key → PutObject
async fn put_object(
    state: S3State,
    bucket: &str,
    key: &str,
    data: Vec<u8>,
) -> Response {
    let bucket_path = PathBuf::from(bucket);
    let object_path = bucket_path.join(key);

    // Ensure bucket exists (auto-create if needed)
    {
        let index = state.file_index.read().unwrap();
        if !index.contains(&bucket_path) {
            drop(index);
            // Auto-create the bucket directory
            let mut index = state.file_index.write().unwrap();
            let mut inode_tbl = state.inode_table.write().unwrap();
            if !index.contains(&bucket_path) {
                let now = SystemTime::now();
                index.insert(bucket_path.clone(), FileEntry {
                    size: 0,
                    is_dir: true,
                    permissions: 0o755,
                    uid: 0,
                    gid: 0,
                    created: now,
                    modified: now,
                    accessed: now,
                    chunks: Vec::new(),
                    symlink_target: None,
                });
                let mut next_ino = state.next_inode.write().unwrap();
                let ino = *next_ino;
                *next_ino += 1;
                inode_tbl.insert(ino, bucket_path.clone());
                info!("S3: Auto-created bucket '{}' for PutObject", bucket);
            }
        }
    }

    // Ensure any parent directories in the key path exist
    if let Some(parent) = object_path.parent() {
        let mut dirs_to_create = Vec::new();
        let mut current = parent.to_path_buf();
        while current != bucket_path && current.components().count() > 0 {
            dirs_to_create.push(current.clone());
            if let Some(p) = current.parent() {
                current = p.to_path_buf();
            } else {
                break;
            }
        }
        dirs_to_create.reverse();

        if !dirs_to_create.is_empty() {
            let mut index = state.file_index.write().unwrap();
            let mut inode_tbl = state.inode_table.write().unwrap();
            for dir in dirs_to_create {
                if !index.contains(&dir) {
                    let now = SystemTime::now();
                    index.insert(dir.clone(), FileEntry {
                        size: 0,
                        is_dir: true,
                        permissions: 0o755,
                        uid: 0,
                        gid: 0,
                        created: now,
                        modified: now,
                        accessed: now,
                        chunks: Vec::new(),
                        symlink_target: None,
                    });
                    let mut next_ino = state.next_inode.write().unwrap();
                    let ino = *next_ino;
                    *next_ino += 1;
                    inode_tbl.insert(ino, dir);
                }
            }
        }
    }

    // Write the object data to chunk store
    let mut chunks: Vec<ChunkRef> = Vec::new();
    let written = match state.chunk_store.write(&mut chunks, 0, &data) {
        Ok(w) => w,
        Err(e) => {
            error!("S3 PutObject: failed to write chunks for {}/{}: {}", bucket, key, e);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalError",
                "Failed to store object data",
            );
        }
    };

    let now = SystemTime::now();
    let entry = FileEntry {
        size: written as u64,
        is_dir: false,
        permissions: 0o644,
        uid: 0,
        gid: 0,
        created: now,
        modified: now,
        accessed: now,
        chunks: chunks.clone(),
        symlink_target: None,
    };

    // Insert into index
    {
        let mut index = state.file_index.write().unwrap();
        let mut inode_tbl = state.inode_table.write().unwrap();

        let old = index.insert(object_path.clone(), entry);

        // Clean up old chunks if overwriting
        if let Some(old_entry) = old {
            if !old_entry.is_dir {
                for chunk in &old_entry.chunks {
                    let _ = state.chunk_store.delete(&chunk.hash);
                }
            }
        }

        // Allocate inode if new
        if inode_tbl.get_inode(&object_path).is_none() {
            let mut next_ino = state.next_inode.write().unwrap();
            let ino = *next_ino;
            *next_ino += 1;
            inode_tbl.insert(ino, object_path);
        }
    }

    let etag = if let Some(chunk) = chunks.first() {
        format!("\"{}\"", hex::encode(&chunk.hash[..16]))
    } else {
        "\"d41d8cd98f00b204e9800998ecf8427e\"".to_string()
    };

    info!("S3 PutObject: {}/{} ({} bytes)", bucket, key, written);

    Response::builder()
        .status(StatusCode::OK)
        .header("ETag", etag)
        .body(Body::empty())
        .unwrap()
}

/// DELETE /bucket/key → DeleteObject
async fn delete_object(state: S3State, bucket: &str, key: &str) -> Response {
    let object_path = PathBuf::from(bucket).join(key);

    let chunks_to_delete = {
        let mut index = state.file_index.write().unwrap();
        let mut inode_tbl = state.inode_table.write().unwrap();

        match index.remove(&object_path) {
            Some(entry) if !entry.is_dir => {
                inode_tbl.remove_path(&object_path);
                entry.chunks
            }
            Some(entry) => {
                // Put it back — can't delete a directory this way
                index.insert(object_path, entry);
                return error_response(StatusCode::CONFLICT, "InvalidRequest", "Cannot delete a directory as an object");
            }
            None => {
                // S3 spec: DELETE on non-existent key returns 204
                Vec::new()
            }
        }
    };

    // Delete chunks
    for chunk in &chunks_to_delete {
        let _ = state.chunk_store.delete(&chunk.hash);
    }

    info!("S3 DeleteObject: {}/{}", bucket, key);
    (StatusCode::NO_CONTENT, [(header::CONTENT_TYPE, "application/xml")]).into_response()
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Parse a request path into (bucket, optional key)
fn parse_bucket_key(path: &str) -> (String, Option<String>) {
    let path = path.trim_start_matches('/');
    if let Some(slash_pos) = path.find('/') {
        let bucket = path[..slash_pos].to_string();
        let key = path[slash_pos + 1..].to_string();
        if key.is_empty() {
            (bucket, None)
        } else {
            (bucket, Some(key))
        }
    } else {
        (path.to_string(), None)
    }
}

/// Check authorization
fn authorize(headers: &HeaderMap, credentials: &Option<S3Credentials>) -> bool {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    check_auth(auth_header, credentials.as_ref())
}

/// Format a SystemTime as ISO 8601 for S3 XML responses
fn format_time(time: &SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs();
    
    // Simple UTC formatting
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Approximate date calculation (good enough for S3 responses)
    let (year, month, day) = days_to_ymd(days);

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z",
        year, month, day, hours, minutes, seconds
    )
}

/// Format a SystemTime as HTTP date (RFC 7231)
fn format_time_http(time: &SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs();

    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let day_of_week = ((days + 4) % 7) as usize; // Jan 1, 1970 was Thursday
    let dow = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

    let (year, month, day) = days_to_ymd(days);

    format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        dow[day_of_week],
        day,
        months[(month - 1) as usize],
        year,
        hours,
        minutes,
        seconds
    )
}

/// Convert days since epoch to (year, month, day)
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Civil calendar calculation from days since epoch
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// XML-escape a string
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Build an S3 error XML response
fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <Error>\n\
         <Code>{}</Code>\n\
         <Message>{}</Message>\n\
         </Error>",
        code, xml_escape(message)
    );

    (
        status,
        [(header::CONTENT_TYPE, "application/xml")],
        xml,
    ).into_response()
}
