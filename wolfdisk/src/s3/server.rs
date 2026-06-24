//! S3-compatible HTTP server for WolfDisk
//!
//! Maps WolfDisk's file index and chunk store to S3 buckets and objects:
//! - Top-level directories → S3 buckets
//! - Files within directories → S3 objects
//!
//! Supports: ListBuckets, ListObjectsV2 (prefix/delimiter), GetObject (+Range),
//! PutObject, HeadObject, DeleteObject, HeadBucket, CreateBucket, DeleteBucket,
//! CopyObject, DeleteObjects (batch), multipart upload (Create/UploadPart/
//! Complete/Abort), and `x-amz-meta-*` user metadata. Authentication is AWS
//! SigV4 (header + presigned) when credentials are configured.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    body::Body,
    extract::{OriginalUri, Path, Query, State},
    http::{header, HeaderMap, Method, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use md5::{Digest, Md5};
use tokio::net::TcpListener;
use tracing::{error, info};

use super::auth::{self, S3Credentials};
use super::meta::{self, MultipartUpload, PartInfo, S3MetaStore, S3ObjectMeta};
use crate::storage::{ChunkRef, ChunkStore, FileEntry, FileIndex, InodeTable};

/// Maximum body buffered in memory for a single PutObject / UploadPart.
const MAX_BODY: usize = 512 * 1024 * 1024;

/// Shared state for the S3 server
#[derive(Clone)]
pub struct S3State {
    pub file_index: Arc<RwLock<FileIndex>>,
    pub chunk_store: Arc<ChunkStore>,
    pub inode_table: Arc<RwLock<InodeTable>>,
    pub next_inode: Arc<RwLock<u64>>,
    pub credentials: Option<S3Credentials>,
    pub region: String,
    /// In-progress multipart uploads (in-memory; uploadId → state).
    pub multipart: Arc<RwLock<HashMap<String, MultipartUpload>>>,
    /// Persistent S3 object metadata sidecar.
    pub meta: Arc<RwLock<S3MetaStore>>,
    /// Path the sidecar is persisted to.
    pub meta_path: PathBuf,
}

/// S3 server that runs alongside WolfDisk FUSE
pub struct S3Server {
    bind_addr: String,
    state: S3State,
}

impl S3Server {
    /// Create a new S3 server. `meta_path` is where S3 object metadata is
    /// persisted (typically `<data_dir>/index/s3_meta.json`).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        bind_addr: String,
        file_index: Arc<RwLock<FileIndex>>,
        chunk_store: Arc<ChunkStore>,
        inode_table: Arc<RwLock<InodeTable>>,
        next_inode: Arc<RwLock<u64>>,
        credentials: Option<S3Credentials>,
        meta_path: PathBuf,
    ) -> Self {
        let meta = S3MetaStore::load(&meta_path);
        let state = S3State {
            file_index,
            chunk_store,
            inode_table,
            next_inode,
            credentials,
            region: "us-east-1".to_string(),
            multipart: Arc::new(RwLock::new(HashMap::new())),
            meta: Arc::new(RwLock::new(meta)),
            meta_path,
        };

        Self { bind_addr, state }
    }

    /// Start the S3 server (call from a tokio runtime)
    pub async fn run(self) -> std::io::Result<()> {
        let app = Router::new()
            .route("/", any(handle_root))
            // axum 0.7 / matchit 0.7 catch-all syntax (`/*path`, not `/{*path}`).
            .route("/*path", any(handle_path))
            .with_state(self.state.clone());

        info!("S3-compatible API listening on {}", self.bind_addr);

        let listener = TcpListener::bind(&self.bind_addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }
}

// ─── Auth wrapper ────────────────────────────────────────────────────────────

/// Verify SigV4. Returns a small `AuthError` (mapped to a response by callers).
fn check(
    state: &S3State,
    method: &Method,
    uri: &axum::http::Uri,
    headers: &HeaderMap,
) -> Result<(), auth::AuthError> {
    auth::verify(
        method.as_str(),
        uri.path(),
        uri.query().unwrap_or(""),
        headers,
        state.credentials.as_ref(),
    )
}

// ─── Root handler (ListBuckets) ──────────────────────────────────────────────

async fn handle_root(
    State(state): State<S3State>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    method: Method,
) -> Response {
    if let Err(e) = check(&state, &method, &uri, &headers) {
        return error_response(e.status, e.code, &e.message);
    }
    match method {
        Method::GET => list_buckets(state).await,
        _ => error_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "MethodNotAllowed",
            "Method not allowed",
        ),
    }
}

// ─── Path handler ────────────────────────────────────────────────────────────

async fn handle_path(
    State(state): State<S3State>,
    OriginalUri(uri): OriginalUri,
    Path(path): Path<String>,
    headers: HeaderMap,
    method: Method,
    query: Query<HashMap<String, String>>,
    request: Request<Body>,
) -> Response {
    if let Err(e) = check(&state, &method, &uri, &headers) {
        return error_response(e.status, e.code, &e.message);
    }

    let (bucket, key) = parse_bucket_key(&path);

    match (method, key) {
        // ── Bucket-level ────────────────────────────────────────
        (Method::GET, None) => {
            if query.contains_key("location") {
                get_bucket_location(state).await
            } else {
                list_objects(state, &bucket, &query).await
            }
        }
        (Method::HEAD, None) => head_bucket(state, &bucket).await,
        (Method::PUT, None) => create_bucket(state, &bucket).await,
        (Method::DELETE, None) => delete_bucket(state, &bucket).await,
        (Method::POST, None) => {
            if query.contains_key("delete") {
                match read_body(request, &headers).await {
                    Ok(body) => delete_objects(state, &bucket, &body).await,
                    Err(resp) => resp,
                }
            } else {
                error_response(
                    StatusCode::METHOD_NOT_ALLOWED,
                    "MethodNotAllowed",
                    "Method not allowed",
                )
            }
        }

        // ── Object-level ────────────────────────────────────────
        (Method::GET, Some(key)) => get_object(state, &bucket, &key, &headers).await,
        (Method::HEAD, Some(key)) => head_object(state, &bucket, &key).await,
        (Method::PUT, Some(key)) => {
            // CopyObject (no body)
            if let Some(src) = headers
                .get("x-amz-copy-source")
                .and_then(|v| v.to_str().ok())
            {
                let src = src.to_string();
                return copy_object(state, &bucket, &key, &src, &headers).await;
            }
            // UploadPart
            if let (Some(pn), Some(uid)) = (query.get("partNumber"), query.get("uploadId")) {
                let pn = pn.clone();
                let uid = uid.clone();
                return match read_body(request, &headers).await {
                    Ok(body) => upload_part(state, &uid, &pn, body).await,
                    Err(resp) => resp,
                };
            }
            // PutObject
            match read_body(request, &headers).await {
                Ok(body) => put_object(state, &bucket, &key, body, &headers).await,
                Err(resp) => resp,
            }
        }
        (Method::POST, Some(key)) => {
            if query.contains_key("uploads") {
                create_multipart(state, &bucket, &key, &headers).await
            } else if let Some(uid) = query.get("uploadId") {
                let uid = uid.clone();
                match read_body(request, &headers).await {
                    Ok(body) => complete_multipart(state, &bucket, &key, &uid, &body).await,
                    Err(resp) => resp,
                }
            } else {
                error_response(
                    StatusCode::METHOD_NOT_ALLOWED,
                    "MethodNotAllowed",
                    "Method not allowed",
                )
            }
        }
        (Method::DELETE, Some(key)) => {
            if let Some(uid) = query.get("uploadId") {
                abort_multipart(state, uid).await
            } else {
                delete_object(state, &bucket, &key).await
            }
        }

        _ => error_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "MethodNotAllowed",
            "Method not allowed",
        ),
    }
}

/// Read (and aws-chunked-decode if needed) a request body.
async fn read_body(request: Request<Body>, headers: &HeaderMap) -> Result<Vec<u8>, Response> {
    let streaming = is_streaming(headers);
    let bytes = match axum::body::to_bytes(request.into_body(), MAX_BODY).await {
        Ok(b) => b,
        Err(e) => {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "InvalidRequest",
                &format!("Failed to read body: {}", e),
            ));
        }
    };
    if streaming {
        auth::decode_aws_chunked(&bytes)
            .map_err(|e| error_response(StatusCode::BAD_REQUEST, "InvalidRequest", &e))
    } else {
        Ok(bytes.to_vec())
    }
}

// ─── Bucket operations ───────────────────────────────────────────────────────

/// GET / → ListBuckets
async fn list_buckets(state: S3State) -> Response {
    let index = state.file_index.read().unwrap();

    let mut buckets: HashSet<String> = HashSet::new();
    for (path, entry) in index.iter() {
        if entry.is_dir {
            if path.components().count() == 1 {
                if let Some(name) = path.file_name() {
                    buckets.insert(name.to_string_lossy().to_string());
                }
            }
        } else if path.components().count() > 1 {
            if let Some(first) = path.components().next() {
                buckets.insert(first.as_os_str().to_string_lossy().to_string());
            }
        }
    }

    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<ListAllMyBucketsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\n");
    xml.push_str(
        "  <Owner>\n    <ID>wolfdisk</ID>\n    <DisplayName>WolfDisk</DisplayName>\n  </Owner>\n",
    );
    xml.push_str("  <Buckets>\n");
    for bucket_name in &buckets {
        xml.push_str("    <Bucket>\n");
        xml.push_str(&format!("      <Name>{}</Name>\n", xml_escape(bucket_name)));
        xml.push_str("      <CreationDate>2025-01-01T00:00:00.000Z</CreationDate>\n");
        xml.push_str("    </Bucket>\n");
    }
    xml.push_str("  </Buckets>\n</ListAllMyBucketsResult>");

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/xml")],
        xml,
    )
        .into_response()
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
    )
        .into_response()
}

/// GET /bucket → ListObjectsV2
async fn list_objects(state: S3State, bucket: &str, query: &HashMap<String, String>) -> Response {
    let prefix = query.get("prefix").cloned().unwrap_or_default();
    let delimiter = query.get("delimiter").cloned().unwrap_or_default();
    let max_keys: usize = query
        .get("max-keys")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);
    let continuation_token = query.get("continuation-token").cloned();

    let index = state.file_index.read().unwrap();
    let bucket_prefix = PathBuf::from(bucket);

    let mut objects: Vec<(String, u64, SystemTime, Vec<ChunkRef>)> = Vec::new();
    let mut common_prefixes: HashSet<String> = HashSet::new();

    for (path, entry) in index.iter() {
        if !path.starts_with(&bucket_prefix) {
            continue;
        }
        let key = path
            .strip_prefix(&bucket_prefix)
            .unwrap()
            .to_string_lossy()
            .to_string();
        if key.is_empty() {
            continue;
        }
        if entry.is_dir {
            if !delimiter.is_empty() {
                let dir_prefix = format!("{}/", key.trim_end_matches('/'));
                if dir_prefix.starts_with(&prefix) {
                    common_prefixes.insert(dir_prefix);
                }
            }
            continue;
        }
        if !key.starts_with(&prefix) {
            continue;
        }
        if !delimiter.is_empty() {
            let after_prefix = &key[prefix.len()..];
            if let Some(delim_pos) = after_prefix.find(&delimiter) {
                let common = format!("{}{}{}", prefix, &after_prefix[..delim_pos], delimiter);
                common_prefixes.insert(common);
                continue;
            }
        }
        objects.push((key, entry.size, entry.modified, entry.chunks.clone()));
    }
    drop(index);

    objects.sort_by(|a, b| a.0.cmp(&b.0));

    if let Some(ref token) = continuation_token {
        if let Some(pos) = objects
            .iter()
            .position(|(k, ..)| k.as_str() > token.as_str())
        {
            objects = objects.split_off(pos);
        } else {
            objects.clear();
        }
    }

    let is_truncated = objects.len() > max_keys;
    let objects: Vec<_> = objects.into_iter().take(max_keys).collect();
    let next_token = if is_truncated {
        objects.last().map(|(k, ..)| k.clone())
    } else {
        None
    };

    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\n");
    xml.push_str(&format!("  <Name>{}</Name>\n", xml_escape(bucket)));
    xml.push_str(&format!("  <Prefix>{}</Prefix>\n", xml_escape(&prefix)));
    xml.push_str(&format!("  <MaxKeys>{}</MaxKeys>\n", max_keys));
    if !delimiter.is_empty() {
        xml.push_str(&format!(
            "  <Delimiter>{}</Delimiter>\n",
            xml_escape(&delimiter)
        ));
    }
    xml.push_str(&format!("  <IsTruncated>{}</IsTruncated>\n", is_truncated));
    xml.push_str(&format!("  <KeyCount>{}</KeyCount>\n", objects.len()));
    if let Some(ref token) = next_token {
        xml.push_str(&format!(
            "  <NextContinuationToken>{}</NextContinuationToken>\n",
            xml_escape(token)
        ));
    }

    let meta = state.meta.read().unwrap();
    for (key, size, modified, chunks) in &objects {
        let etag = object_etag(&meta, bucket, key, chunks);
        xml.push_str("  <Contents>\n");
        xml.push_str(&format!("    <Key>{}</Key>\n", xml_escape(key)));
        xml.push_str(&format!("    <Size>{}</Size>\n", size));
        xml.push_str(&format!(
            "    <LastModified>{}</LastModified>\n",
            format_time(modified)
        ));
        xml.push_str(&format!("    <ETag>\"{}\"</ETag>\n", etag));
        xml.push_str("    <StorageClass>STANDARD</StorageClass>\n  </Contents>\n");
    }
    drop(meta);

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
    )
        .into_response()
}

/// HEAD /bucket → HeadBucket
async fn head_bucket(state: S3State, bucket: &str) -> Response {
    let index = state.file_index.read().unwrap();
    let bucket_path = PathBuf::from(bucket);
    let exists = index.get(&bucket_path).map(|e| e.is_dir).unwrap_or(false)
        || index.iter().any(|(p, _)| p.starts_with(&bucket_path));
    if exists {
        (StatusCode::OK, [(header::CONTENT_TYPE, "application/xml")]).into_response()
    } else {
        error_response(
            StatusCode::NOT_FOUND,
            "NoSuchBucket",
            "The specified bucket does not exist",
        )
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
                "BucketAlreadyOwnedByYou",
                "The requested bucket already exists",
            );
        }
        index.insert(bucket_path.clone(), new_dir_entry());
        let mut next_ino = state.next_inode.write().unwrap();
        let ino = *next_ino;
        *next_ino += 1;
        inode_tbl.insert(ino, bucket_path);
    }
    info!("S3: Created bucket '{}'", bucket);
    (StatusCode::OK, [(header::CONTENT_TYPE, "application/xml")]).into_response()
}

/// DELETE /bucket → DeleteBucket. A bucket with no *objects* is deletable;
/// synthetic intermediate directories are swept away with it (S3 has no dirs).
async fn delete_bucket(state: S3State, bucket: &str) -> Response {
    let bucket_path = PathBuf::from(bucket);
    {
        let mut index = state.file_index.write().unwrap();
        let mut inode_tbl = state.inode_table.write().unwrap();

        match index.get(&bucket_path) {
            Some(e) if !e.is_dir => {
                return error_response(StatusCode::NOT_FOUND, "NoSuchBucket", "Not a bucket");
            }
            None => {
                return error_response(StatusCode::NOT_FOUND, "NoSuchBucket", "Bucket not found");
            }
            _ => {}
        }

        let mut to_remove: Vec<PathBuf> = Vec::new();
        let mut has_objects = false;
        for (p, e) in index.iter() {
            if *p == bucket_path {
                to_remove.push(p.clone());
            } else if p.starts_with(&bucket_path) {
                if !e.is_dir {
                    has_objects = true;
                }
                to_remove.push(p.clone());
            }
        }
        if has_objects {
            return error_response(
                StatusCode::CONFLICT,
                "BucketNotEmpty",
                "The bucket is not empty",
            );
        }
        for p in &to_remove {
            index.remove(p);
            inode_tbl.remove_path(p);
        }
    }
    info!("S3: Deleted bucket '{}'", bucket);
    (
        StatusCode::NO_CONTENT,
        [(header::CONTENT_TYPE, "application/xml")],
    )
        .into_response()
}

// ─── Object operations ───────────────────────────────────────────────────────

/// GET /bucket/key → GetObject (supports Range)
async fn get_object(state: S3State, bucket: &str, key: &str, headers: &HeaderMap) -> Response {
    let object_path = PathBuf::from(bucket).join(key);

    let entry = {
        let index = state.file_index.read().unwrap();
        match index.get(&object_path) {
            Some(e) if !e.is_dir => e.clone(),
            Some(_) => {
                return error_response(StatusCode::NOT_FOUND, "NoSuchKey", "Key is a directory")
            }
            None => {
                return error_response(
                    StatusCode::NOT_FOUND,
                    "NoSuchKey",
                    "The specified key does not exist",
                )
            }
        }
    };

    let (offset, length, partial) = match parse_range(headers, entry.size) {
        Some(Ok((start, end))) => (start, (end - start + 1) as usize, true),
        Some(Err(())) => {
            return error_response(
                StatusCode::RANGE_NOT_SATISFIABLE,
                "InvalidRange",
                "The requested range is not satisfiable",
            )
        }
        None => (0u64, entry.size as usize, false),
    };

    let data = match state.chunk_store.read(&entry.chunks, offset, length) {
        Ok(d) => d,
        Err(e) => {
            error!("S3 GetObject: failed to read {}/{}: {}", bucket, key, e);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalError",
                "Failed to read object data",
            );
        }
    };

    let m = state.meta.read().unwrap();
    let obj_meta = m.get(&meta_key(bucket, key)).cloned().unwrap_or_default();
    drop(m);
    let etag = obj_meta
        .etag
        .unwrap_or_else(|| first_chunk_etag(&entry.chunks));
    let content_type = obj_meta
        .content_type
        .unwrap_or_else(|| "application/octet-stream".to_string());

    let mut builder = Response::builder();
    if partial {
        builder = builder.status(StatusCode::PARTIAL_CONTENT).header(
            header::CONTENT_RANGE,
            format!(
                "bytes {}-{}/{}",
                offset,
                offset + length as u64 - 1,
                entry.size
            ),
        );
    } else {
        builder = builder.status(StatusCode::OK);
    }
    builder = builder
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_LENGTH, data.len().to_string())
        .header(header::ACCEPT_RANGES, "bytes")
        .header("ETag", format!("\"{}\"", etag))
        .header("Last-Modified", format_time_http(&entry.modified));
    for (k, v) in &obj_meta.user_meta {
        builder = builder.header(format!("x-amz-meta-{}", k), v.clone());
    }
    builder.body(Body::from(data)).unwrap()
}

/// HEAD /bucket/key → HeadObject
async fn head_object(state: S3State, bucket: &str, key: &str) -> Response {
    let object_path = PathBuf::from(bucket).join(key);
    let index = state.file_index.read().unwrap();
    let entry = match index.get(&object_path) {
        Some(e) if !e.is_dir => e.clone(),
        _ => return error_response(StatusCode::NOT_FOUND, "NoSuchKey", "Key not found"),
    };
    drop(index);

    let m = state.meta.read().unwrap();
    let obj_meta = m.get(&meta_key(bucket, key)).cloned().unwrap_or_default();
    drop(m);
    let etag = obj_meta
        .etag
        .unwrap_or_else(|| first_chunk_etag(&entry.chunks));
    let content_type = obj_meta
        .content_type
        .unwrap_or_else(|| "application/octet-stream".to_string());

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_LENGTH, entry.size.to_string())
        .header(header::ACCEPT_RANGES, "bytes")
        .header("ETag", format!("\"{}\"", etag))
        .header("Last-Modified", format_time_http(&entry.modified));
    for (k, v) in &obj_meta.user_meta {
        builder = builder.header(format!("x-amz-meta-{}", k), v.clone());
    }
    builder.body(Body::empty()).unwrap()
}

/// PUT /bucket/key → PutObject
async fn put_object(
    state: S3State,
    bucket: &str,
    key: &str,
    data: Vec<u8>,
    headers: &HeaderMap,
) -> Response {
    let object_path = PathBuf::from(bucket).join(key);
    ensure_bucket_and_parents(&state, bucket, &object_path);

    let mut chunks: Vec<ChunkRef> = Vec::new();
    let written = match state.chunk_store.write(&mut chunks, 0, &data) {
        Ok(w) => w,
        Err(e) => {
            error!("S3 PutObject: failed to write {}/{}: {}", bucket, key, e);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalError",
                "Failed to store object data",
            );
        }
    };

    let etag = hex::encode(Md5::digest(&data));
    store_object(&state, &object_path, chunks, written as u64);
    set_object_meta(
        &state,
        bucket,
        key,
        S3ObjectMeta {
            content_type: content_type_of(headers),
            etag: Some(etag.clone()),
            user_meta: extract_user_meta(headers),
        },
    );

    info!("S3 PutObject: {}/{} ({} bytes)", bucket, key, written);
    Response::builder()
        .status(StatusCode::OK)
        .header("ETag", format!("\"{}\"", etag))
        .body(Body::empty())
        .unwrap()
}

/// DELETE /bucket/key → DeleteObject (idempotent)
async fn delete_object(state: S3State, bucket: &str, key: &str) -> Response {
    let object_path = PathBuf::from(bucket).join(key);
    match remove_object(&state, &object_path) {
        Ok(()) => {
            set_object_meta_remove(&state, bucket, key);
            info!("S3 DeleteObject: {}/{}", bucket, key);
            (
                StatusCode::NO_CONTENT,
                [(header::CONTENT_TYPE, "application/xml")],
            )
                .into_response()
        }
        Err(()) => error_response(
            StatusCode::CONFLICT,
            "InvalidRequest",
            "Cannot delete a directory as an object",
        ),
    }
}

/// PUT /bucket/key with x-amz-copy-source → CopyObject
async fn copy_object(
    state: S3State,
    dst_bucket: &str,
    dst_key: &str,
    raw_source: &str,
    headers: &HeaderMap,
) -> Response {
    // Source format: "/srcbucket/srckey" or "srcbucket/srckey", maybe url-encoded, maybe ?versionId.
    let src = raw_source.split('?').next().unwrap_or(raw_source);
    let src = src.strip_prefix('/').unwrap_or(src);
    let src = percent_encoding::percent_decode_str(src)
        .decode_utf8_lossy()
        .to_string();
    let (src_bucket, src_key) = match src.split_once('/') {
        Some((b, k)) if !k.is_empty() => (b.to_string(), k.to_string()),
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "InvalidArgument",
                "Malformed x-amz-copy-source",
            )
        }
    };

    let src_path = PathBuf::from(&src_bucket).join(&src_key);
    let src_entry = {
        let index = state.file_index.read().unwrap();
        match index.get(&src_path) {
            Some(e) if !e.is_dir => e.clone(),
            _ => return error_response(StatusCode::NOT_FOUND, "NoSuchKey", "Source key not found"),
        }
    };

    let dst_path = PathBuf::from(dst_bucket).join(dst_key);
    let replace = headers
        .get("x-amz-metadata-directive")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("REPLACE"))
        .unwrap_or(false);
    if src_path == dst_path && !replace {
        return error_response(
            StatusCode::BAD_REQUEST,
            "InvalidRequest",
            "Copy destination is the same as source without REPLACE directive",
        );
    }

    ensure_bucket_and_parents(&state, dst_bucket, &dst_path);
    // Share chunks by reference (content-addressed); refcount-safe deletes
    // protect against later corruption.
    store_object(&state, &dst_path, src_entry.chunks.clone(), src_entry.size);

    // Determine metadata.
    let src_meta = {
        let m = state.meta.read().unwrap();
        m.get(&meta_key(&src_bucket, &src_key))
            .cloned()
            .unwrap_or_default()
    };
    let etag = src_meta
        .etag
        .clone()
        .unwrap_or_else(|| first_chunk_etag(&src_entry.chunks));
    let new_meta = if replace {
        S3ObjectMeta {
            content_type: content_type_of(headers),
            etag: Some(etag.clone()),
            user_meta: extract_user_meta(headers),
        }
    } else {
        S3ObjectMeta {
            content_type: src_meta.content_type.clone(),
            etag: Some(etag.clone()),
            user_meta: src_meta.user_meta.clone(),
        }
    };
    set_object_meta(&state, dst_bucket, dst_key, new_meta);

    let now = SystemTime::now();
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <CopyObjectResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
         <LastModified>{}</LastModified><ETag>\"{}\"</ETag></CopyObjectResult>",
        format_time(&now),
        xml_escape(&etag)
    );
    info!(
        "S3 CopyObject: {}/{} -> {}/{}",
        src_bucket, src_key, dst_bucket, dst_key
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/xml")],
        xml,
    )
        .into_response()
}

/// POST /bucket?delete → DeleteObjects (batch)
async fn delete_objects(state: S3State, bucket: &str, body: &[u8]) -> Response {
    let (keys, quiet) = match meta::parse_delete(body) {
        Ok(v) => v,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, "MalformedXML", &e),
    };

    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<DeleteResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\n");
    for key in keys {
        let object_path = PathBuf::from(bucket).join(&key);
        match remove_object(&state, &object_path) {
            Ok(()) => {
                set_object_meta_remove(&state, bucket, &key);
                if !quiet {
                    xml.push_str(&format!(
                        "  <Deleted><Key>{}</Key></Deleted>\n",
                        xml_escape(&key)
                    ));
                }
            }
            Err(_) => {
                xml.push_str(&format!(
                    "  <Error><Key>{}</Key><Code>InternalError</Code><Message>delete failed</Message></Error>\n",
                    xml_escape(&key)
                ));
            }
        }
    }
    xml.push_str("</DeleteResult>");
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/xml")],
        xml,
    )
        .into_response()
}

// ─── Multipart upload ────────────────────────────────────────────────────────

/// POST /bucket/key?uploads → CreateMultipartUpload
async fn create_multipart(
    state: S3State,
    bucket: &str,
    key: &str,
    headers: &HeaderMap,
) -> Response {
    let upload_id = hex::encode(rand::random::<[u8; 16]>());
    let upload = MultipartUpload {
        bucket: bucket.to_string(),
        key: key.to_string(),
        content_type: content_type_of(headers),
        user_meta: extract_user_meta(headers),
        parts: BTreeMap::new(),
    };
    state
        .multipart
        .write()
        .unwrap()
        .insert(upload_id.clone(), upload);

    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <InitiateMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
         <Bucket>{}</Bucket><Key>{}</Key><UploadId>{}</UploadId></InitiateMultipartUploadResult>",
        xml_escape(bucket),
        xml_escape(key),
        xml_escape(&upload_id)
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/xml")],
        xml,
    )
        .into_response()
}

/// PUT /bucket/key?partNumber=&uploadId= → UploadPart
async fn upload_part(
    state: S3State,
    upload_id: &str,
    part_number: &str,
    data: Vec<u8>,
) -> Response {
    let part_number: u32 = match part_number.parse() {
        Ok(n) if (1..=10_000).contains(&n) => n,
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "InvalidArgument",
                "Invalid partNumber",
            )
        }
    };

    let mut chunks: Vec<ChunkRef> = Vec::new();
    if let Err(e) = state.chunk_store.write(&mut chunks, 0, &data) {
        error!("S3 UploadPart: chunk write failed: {}", e);
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "InternalError",
            "Failed to store part",
        );
    }
    let md5_hex = hex::encode(Md5::digest(&data));

    let old_chunks = {
        let mut uploads = state.multipart.write().unwrap();
        let upload = match uploads.get_mut(upload_id) {
            Some(u) => u,
            None => {
                return error_response(
                    StatusCode::NOT_FOUND,
                    "NoSuchUpload",
                    "The specified upload does not exist",
                )
            }
        };
        upload
            .parts
            .insert(
                part_number,
                PartInfo {
                    chunks,
                    md5_hex: md5_hex.clone(),
                    size: data.len() as u64,
                },
            )
            .map(|p| p.chunks)
    };
    // Free chunks of a replaced part (refcount-safe).
    if let Some(old) = old_chunks {
        free_chunks_unreferenced(&state, &old);
    }

    Response::builder()
        .status(StatusCode::OK)
        .header("ETag", format!("\"{}\"", md5_hex))
        .body(Body::empty())
        .unwrap()
}

/// POST /bucket/key?uploadId= → CompleteMultipartUpload
async fn complete_multipart(
    state: S3State,
    bucket: &str,
    key: &str,
    upload_id: &str,
    body: &[u8],
) -> Response {
    let requested = match meta::parse_complete_multipart(body) {
        Ok(v) => v,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, "MalformedXML", &e),
    };

    let upload = {
        let uploads = state.multipart.read().unwrap();
        match uploads.get(upload_id) {
            Some(u) => u.clone(),
            None => {
                return error_response(
                    StatusCode::NOT_FOUND,
                    "NoSuchUpload",
                    "The specified upload does not exist",
                )
            }
        }
    };

    // Assemble final chunk list and the multipart ETag.
    let mut final_chunks: Vec<ChunkRef> = Vec::new();
    let mut md5_concat: Vec<u8> = Vec::new();
    let mut total: u64 = 0;
    let mut last = 0u32;
    for (num, etag) in &requested {
        if *num <= last {
            return error_response(
                StatusCode::BAD_REQUEST,
                "InvalidPartOrder",
                "Parts must be in ascending order",
            );
        }
        last = *num;
        let part = match upload.parts.get(num) {
            Some(p) => p,
            None => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "InvalidPart",
                    &format!("Missing part {}", num),
                )
            }
        };
        if !etag.trim_matches('"').eq_ignore_ascii_case(&part.md5_hex) {
            return error_response(
                StatusCode::BAD_REQUEST,
                "InvalidPart",
                &format!("ETag mismatch for part {}", num),
            );
        }
        for c in &part.chunks {
            final_chunks.push(ChunkRef {
                hash: c.hash,
                offset: total + c.offset,
                size: c.size,
            });
        }
        match hex::decode(&part.md5_hex) {
            Ok(bytes) => md5_concat.extend_from_slice(&bytes),
            Err(_) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "InternalError",
                    "Bad part checksum",
                )
            }
        }
        total += part.size;
    }
    let etag = format!(
        "{}-{}",
        hex::encode(Md5::digest(&md5_concat)),
        requested.len()
    );

    let object_path = PathBuf::from(bucket).join(key);
    ensure_bucket_and_parents(&state, bucket, &object_path);
    store_object(&state, &object_path, final_chunks, total);
    set_object_meta(
        &state,
        bucket,
        key,
        S3ObjectMeta {
            content_type: upload.content_type.clone(),
            etag: Some(etag.clone()),
            user_meta: upload.user_meta.clone(),
        },
    );

    // Remove the upload from the registry (parts are now part of the object).
    state.multipart.write().unwrap().remove(upload_id);

    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <CompleteMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
         <Location>/{}/{}</Location><Bucket>{}</Bucket><Key>{}</Key><ETag>\"{}\"</ETag>\
         </CompleteMultipartUploadResult>",
        xml_escape(bucket),
        xml_escape(key),
        xml_escape(bucket),
        xml_escape(key),
        xml_escape(&etag)
    );
    info!(
        "S3 CompleteMultipartUpload: {}/{} ({} bytes, {} parts)",
        bucket,
        key,
        total,
        requested.len()
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/xml")],
        xml,
    )
        .into_response()
}

/// DELETE /bucket/key?uploadId= → AbortMultipartUpload
async fn abort_multipart(state: S3State, upload_id: &str) -> Response {
    let removed = state.multipart.write().unwrap().remove(upload_id);
    match removed {
        Some(upload) => {
            for part in upload.parts.values() {
                free_chunks_unreferenced(&state, &part.chunks);
            }
            (
                StatusCode::NO_CONTENT,
                [(header::CONTENT_TYPE, "application/xml")],
            )
                .into_response()
        }
        None => error_response(
            StatusCode::NOT_FOUND,
            "NoSuchUpload",
            "The specified upload does not exist",
        ),
    }
}

// ─── Shared object/storage helpers ───────────────────────────────────────────

fn new_dir_entry() -> FileEntry {
    let now = SystemTime::now();
    FileEntry {
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
    }
}

/// Auto-create the bucket directory and any intermediate directories for an
/// object path (mirrors the FUSE-visible namespace).
fn ensure_bucket_and_parents(state: &S3State, bucket: &str, object_path: &std::path::Path) {
    let bucket_path = PathBuf::from(bucket);
    let mut index = state.file_index.write().unwrap();
    let mut inode_tbl = state.inode_table.write().unwrap();

    let mut to_create: Vec<PathBuf> = Vec::new();
    if !index.contains(&bucket_path) {
        to_create.push(bucket_path.clone());
    }
    if let Some(parent) = object_path.parent() {
        let mut cur = parent.to_path_buf();
        while cur != bucket_path && cur.components().count() > 0 {
            if !index.contains(&cur) {
                to_create.push(cur.clone());
            }
            match cur.parent() {
                Some(p) => cur = p.to_path_buf(),
                None => break,
            }
        }
    }
    // Create shallowest first.
    to_create.sort_by_key(|p| p.components().count());
    for dir in to_create {
        if !index.contains(&dir) {
            index.insert(dir.clone(), new_dir_entry());
            let mut next_ino = state.next_inode.write().unwrap();
            let ino = *next_ino;
            *next_ino += 1;
            inode_tbl.insert(ino, dir);
        }
    }
}

/// Insert a file entry, allocating an inode and freeing any orphaned chunks of
/// a replaced entry (reference-counted, dedup-safe).
fn store_object(state: &S3State, object_path: &std::path::Path, chunks: Vec<ChunkRef>, size: u64) {
    let now = SystemTime::now();
    let entry = FileEntry {
        size,
        is_dir: false,
        permissions: 0o644,
        uid: 0,
        gid: 0,
        created: now,
        modified: now,
        accessed: now,
        chunks,
        symlink_target: None,
    };

    let mut index = state.file_index.write().unwrap();
    let mut inode_tbl = state.inode_table.write().unwrap();
    let old = index.insert(object_path.to_path_buf(), entry);
    if inode_tbl.get_inode(&object_path.to_path_buf()).is_none() {
        let mut next_ino = state.next_inode.write().unwrap();
        let ino = *next_ino;
        *next_ino += 1;
        inode_tbl.insert(ino, object_path.to_path_buf());
    }
    if let Some(old) = old {
        if !old.is_dir {
            free_chunks_with_index(state, &index, &old.chunks);
        }
    }
}

/// Remove an object entry (idempotent), freeing orphaned chunks. Returns
/// `Err(())` only when the path names a directory (not deletable as an object).
fn remove_object(state: &S3State, object_path: &std::path::Path) -> Result<(), ()> {
    let mut index = state.file_index.write().unwrap();
    let mut inode_tbl = state.inode_table.write().unwrap();
    match index.remove(object_path) {
        Some(entry) if !entry.is_dir => {
            inode_tbl.remove_path(&object_path.to_path_buf());
            free_chunks_with_index(state, &index, &entry.chunks);
            Ok(())
        }
        Some(entry) => {
            index.insert(object_path.to_path_buf(), entry);
            Err(())
        }
        None => Ok(()),
    }
}

/// Delete each chunk not referenced by any remaining index entry OR any
/// in-flight multipart part. Takes an already-held index reference (lock order:
/// file_index before multipart). Reference-counted so deduplicated chunks shared
/// by other objects/parts are never deleted out from under them.
fn free_chunks_with_index(state: &S3State, index: &FileIndex, chunks: &[ChunkRef]) {
    let uploads = state.multipart.read().unwrap();
    let mut seen: HashSet<[u8; 32]> = HashSet::new();
    for c in chunks {
        if !seen.insert(c.hash) {
            continue;
        }
        let in_index = index
            .iter()
            .any(|(_, e)| e.chunks.iter().any(|x| x.hash == c.hash));
        let in_parts = uploads.values().any(|u| {
            u.parts
                .values()
                .any(|p| p.chunks.iter().any(|x| x.hash == c.hash))
        });
        if !in_index && !in_parts {
            let _ = state.chunk_store.delete(&c.hash);
        }
    }
}

/// Same as `free_chunks_with_index` but acquires the index read lock itself
/// (used when no index guard is held, e.g. multipart part replacement/abort).
fn free_chunks_unreferenced(state: &S3State, chunks: &[ChunkRef]) {
    let index = state.file_index.read().unwrap();
    free_chunks_with_index(state, &index, chunks);
}

fn set_object_meta(state: &S3State, bucket: &str, key: &str, m: S3ObjectMeta) {
    {
        let mut store = state.meta.write().unwrap();
        store.put(meta_key(bucket, key), m);
    }
    persist_meta(state);
}

fn set_object_meta_remove(state: &S3State, bucket: &str, key: &str) {
    {
        let mut store = state.meta.write().unwrap();
        store.remove(&meta_key(bucket, key));
    }
    persist_meta(state);
}

fn persist_meta(state: &S3State) {
    let store = state.meta.read().unwrap();
    if let Err(e) = store.save(&state.meta_path) {
        error!("S3: failed to persist metadata sidecar: {}", e);
    }
}

fn meta_key(bucket: &str, key: &str) -> String {
    format!("{}/{}", bucket, key)
}

/// ETag for listings: prefer the stored S3 ETag, else fall back to the legacy
/// first-chunk scheme.
fn object_etag(meta: &S3MetaStore, bucket: &str, key: &str, chunks: &[ChunkRef]) -> String {
    meta.get(&meta_key(bucket, key))
        .and_then(|m| m.etag.clone())
        .unwrap_or_else(|| first_chunk_etag(chunks))
}

fn first_chunk_etag(chunks: &[ChunkRef]) -> String {
    match chunks.first() {
        Some(c) => hex::encode(&c.hash[..16]),
        None => "d41d8cd98f00b204e9800998ecf8427e".to_string(),
    }
}

fn content_type_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty() && s != "application/octet-stream")
}

fn extract_user_meta(headers: &HeaderMap) -> HashMap<String, String> {
    let mut meta = HashMap::new();
    for (name, value) in headers.iter() {
        if let Some(k) = name.as_str().strip_prefix("x-amz-meta-") {
            if let Ok(v) = value.to_str() {
                meta.insert(k.to_string(), v.to_string());
            }
        }
    }
    meta
}

fn is_streaming(headers: &HeaderMap) -> bool {
    let sha_streaming = headers
        .get("x-amz-content-sha256")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.starts_with("STREAMING-"))
        .unwrap_or(false);
    let enc_chunked = headers
        .get(header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.contains("aws-chunked"))
        .unwrap_or(false);
    sha_streaming || enc_chunked
}

/// Parse a Range header into an inclusive (start, end), clamped to `size`.
/// None = no range; Some(Err) = unsatisfiable.
fn parse_range(headers: &HeaderMap, size: u64) -> Option<Result<(u64, u64), ()>> {
    let raw = headers.get(header::RANGE)?.to_str().ok()?;
    let spec = raw.strip_prefix("bytes=")?;
    let spec = spec.split(',').next().unwrap_or(spec).trim();
    if size == 0 {
        return Some(Err(()));
    }
    let (start_s, end_s) = spec.split_once('-')?;
    let result = if start_s.is_empty() {
        match end_s.parse::<u64>() {
            Ok(n) if n > 0 => {
                let n = n.min(size);
                Ok((size - n, size - 1))
            }
            _ => Err(()),
        }
    } else {
        let start: u64 = match start_s.parse() {
            Ok(v) => v,
            Err(_) => return Some(Err(())),
        };
        if start >= size {
            return Some(Err(()));
        }
        let end = if end_s.is_empty() {
            size - 1
        } else {
            match end_s.parse::<u64>() {
                Ok(v) => v.min(size - 1),
                Err(_) => return Some(Err(())),
            }
        };
        if end < start {
            Err(())
        } else {
            Ok((start, end))
        }
    };
    Some(result)
}

// ─── Generic helpers (unchanged) ─────────────────────────────────────────────

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

fn format_time(time: &SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs();
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;
    let (year, month, day) = days_to_ymd(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z",
        year, month, day, hours, minutes, seconds
    )
}

fn format_time_http(time: &SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs();
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;
    let day_of_week = ((days + 4) % 7) as usize;
    let dow = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
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

fn days_to_ymd(days: u64) -> (u64, u64, u64) {
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

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <Error>\n<Code>{}</Code>\n<Message>{}</Message>\n</Error>",
        code,
        xml_escape(message)
    );
    (status, [(header::CONTENT_TYPE, "application/xml")], xml).into_response()
}
