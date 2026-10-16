//! Queued downloads: concurrency limit, retry, hash verification, resume.
//!
//! Mirrors `download_manager.py` + `artifact_verifier.py` behaviour:
//! `.part` sibling files, verify size then HTML then hashes, atomic commit.
//! Cancel is a *status*, never an error (spec §12): it keeps the `.part`
//! for resume (`F-035`); only verification failure deletes it (`F-034`).

use sha1::Digest as _;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Semaphore;

use crate::downloader::retry;
use crate::safety::atomic::{self, AtomicError};

/// Redirects the old app follows by hand (`downloads.md` §7).
const REDIRECT_CODES: &[u16] = &[301, 302, 303, 307, 308];
const MAX_REDIRECTS: u32 = 10;
/// Pause poll interval (`download_manager.py` sleeps 0.15 s in its pause loop).
const PAUSE_POLL_MS: u64 = 150;
/// HTML sniff window: first bytes after stripping BOM/NUL/whitespace (spec §9).
const HTML_SNIFF_LEN: usize = 1024;

/// What the caller expects, when known.
#[derive(Debug, Clone, Default)]
pub struct DownloadRequest {
    pub url: String,
    pub dest: PathBuf,
    pub expected_size: Option<u64>,
    pub sha1_hex: Option<String>,
    pub sha512_hex: Option<String>,
    /// Tests serve `http://127.0.0.1` (wiremock); production stays https-only.
    pub allow_insecure_loopback: bool,
}

/// Progress phases, mirroring the old `downloading→verifying→committing` states.
///
/// Throttling to 100–250 ms is the Tauri Channel layer's job (§5.7), not
/// core's: this callback fires per chunk so no progress is ever lost here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadPhase {
    Downloading,
    Verifying,
    Committing,
}

#[derive(Debug, Clone, Copy)]
pub struct DownloadProgress {
    pub phase: DownloadPhase,
    pub done: u64,
    pub total: Option<u64>,
}

/// Terminal status. Cancelled carries the `.part` path for resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStatus {
    Success,
    Skipped,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct DownloadOutcome {
    pub status: DownloadStatus,
    pub path: PathBuf,
}

#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("invalid url: {0}")]
    InvalidUrl(String),
    #[error("internal: {0}")]
    Internal(&'static str),
    #[error("redirect limit exceeded")]
    RedirectLimit,
    #[error("redirect without location")]
    RedirectNoLocation,
    #[error("http {status}")]
    Http {
        status: u16,
        retry_after: Option<String>,
    },
    #[error("size mismatch")]
    SizeMismatch,
    #[error("hash mismatch")]
    HashMismatch,
    #[error("html rejected")]
    HtmlRejected,
    #[error("not enough disk space")]
    NoSpace,
    #[error("transport: {0}")]
    Transport(String),
    #[error(transparent)]
    Atomic(#[from] AtomicError),
    #[error("io {op} on {}", path.display())]
    Io {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl DownloadError {
    /// Retryable per downloads.md §6-1, delegating to the spec-pinned policy.
    fn retryable(&self) -> bool {
        match self {
            DownloadError::Transport(_) => retry::should_retry(None),
            DownloadError::Http { status, .. } => retry::should_retry(Some(*status)),
            _ => false,
        }
    }

    fn retry_headers(&self) -> reqwest::header::HeaderMap {
        let mut hm = reqwest::header::HeaderMap::new();
        if let DownloadError::Http {
            retry_after: Some(ra),
            ..
        } = self
        {
            if let Ok(v) = ra.parse() {
                hm.insert(reqwest::header::RETRY_AFTER, v);
            }
        }
        hm
    }
}

pub struct Downloader {
    client: reqwest::Client,
    sem: Arc<Semaphore>,
    policy: retry::RetryPolicy,
    space_probe: fn(&Path) -> std::io::Result<u64>,
}

impl Downloader {
    pub fn new(concurrency: usize) -> Result<Self, DownloadError> {
        Self::with_policy(concurrency, retry::RetryPolicy::default())
    }

    pub fn with_policy(
        concurrency: usize,
        policy: retry::RetryPolicy,
    ) -> Result<Self, DownloadError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(crate::USER_AGENT)
            .build()
            .map_err(|_| DownloadError::Internal("client build"))?;
        Ok(Self {
            client,
            // F-039: parallelism clamped to 1–8.
            sem: Arc::new(Semaphore::new(concurrency.clamp(1, 8))),
            policy,
            space_probe: real_free_space,
        })
    }

    #[cfg(test)]
    fn set_space_probe(&mut self, probe: fn(&Path) -> std::io::Result<u64>) {
        self.space_probe = probe;
    }

    pub async fn download(
        &self,
        req: &DownloadRequest,
        on_progress: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
        is_paused: &dyn Fn() -> bool,
    ) -> Result<DownloadOutcome, DownloadError> {
        validate_url(&req.url, req.allow_insecure_loopback)?;
        // F-032/F-033: an existing dest is never silently overwritten.
        let mut req = req.clone();
        if req.dest.exists() {
            if (req.sha1_hex.is_some() || req.sha512_hex.is_some())
                && check_file(&req.dest, &req).await.is_ok()
            {
                return Ok(DownloadOutcome {
                    status: DownloadStatus::Skipped,
                    path: req.dest.clone(),
                });
            }
            req.dest = unique_path(&req.dest);
        }
        // Permit held for the whole download: the concurrency limit.
        let _permit = self
            .sem
            .acquire()
            .await
            .map_err(|_| DownloadError::Internal("semaphore closed"))?;
        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            match self.try_once(&req, on_progress, is_paused).await {
                Ok(o) => return Ok(o),
                Err(e) if e.retryable() && attempt < self.policy.max_attempts => {
                    let wait = retry::retry_delay_secs(attempt, &e.retry_headers(), &self.policy);
                    tokio::time::sleep(std::time::Duration::from_secs_f64(wait)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// One attempt: redirects, resume, stream, verify, commit.
    async fn try_once(
        &self,
        req: &DownloadRequest,
        on_progress: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
        is_paused: &dyn Fn() -> bool,
    ) -> Result<DownloadOutcome, DownloadError> {
        use reqwest::header::{CONTENT_LENGTH, LOCATION, RANGE};
        use reqwest::StatusCode;

        let part = atomic::part_path(&req.dest);
        if let Some(parent) = req.dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_err("create_dir", parent, e))?;
        }
        // Spec §8 path guard: a pre-placed symlink at the part path is
        // removed (the link only) and the part starts fresh.
        if part
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            std::fs::remove_file(&part).map_err(|e| io_err("remove", &part, e))?;
        }
        // Resume state from the existing `.part` (F-035): oversized part is
        // discarded, matching part proceeds to verification below.
        let mut resumed: u64 = 0;
        if let Ok(m) = std::fs::metadata(&part) {
            let len = m.len();
            match req.expected_size {
                Some(e) if len > e => {
                    return Err(remove_part_quiet(&part, DownloadError::SizeMismatch));
                }
                Some(e) if len == e && e > 0 => {
                    return self.verify_and_commit(req, None, on_progress).await;
                }
                _ => resumed = len,
            }
        }
        // Disk preflight is advisory (TOCTOU): unknown size skips the check.
        if let Some(e) = req.expected_size {
            if e > resumed {
                let free =
                    (self.space_probe)(&req.dest).map_err(|e| io_err("space", &req.dest, e))?;
                if free < e - resumed {
                    return Err(DownloadError::NoSpace);
                }
            }
        }
        // Manual redirect loop: every hop is validated before requesting.
        let mut cur = validate_url(&req.url, req.allow_insecure_loopback)?;
        let mut response;
        let mut hops: u32 = 0;
        let mut restarted_416 = false;
        // 416 restart re-enters the request without Range (F-026 pattern).
        let mut want_range = resumed;
        'request: loop {
            let mut rb = self.client.get(cur.clone());
            if want_range > 0 {
                rb = rb.header(RANGE, format!("bytes={want_range}-"));
            }
            let resp = rb.send().await.map_err(transport)?;
            let status = resp.status();
            if REDIRECT_CODES.contains(&status.as_u16()) {
                let loc = resp
                    .headers()
                    .get(LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or(DownloadError::RedirectNoLocation)?;
                hops += 1;
                if hops >= MAX_REDIRECTS {
                    return Err(DownloadError::RedirectLimit);
                }
                cur = cur
                    .join(loc)
                    .map_err(|_| DownloadError::InvalidUrl(loc.to_string()))?;
                validate_url(cur.as_str(), req.allow_insecure_loopback)?;
                continue;
            }
            if status == StatusCode::RANGE_NOT_SATISFIABLE {
                if want_range > 0 && !restarted_416 {
                    restarted_416 = true;
                    match std::fs::remove_file(&part) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(io_err("remove", &part, e)),
                    }
                    want_range = 0;
                    resumed = 0;
                    continue 'request;
                }
                return Err(remove_part_quiet(&part, DownloadError::SizeMismatch));
            }
            response = resp;
            break;
        }
        let status = response.status();
        if !(status.is_success()) {
            return Err(DownloadError::Http {
                status: status.as_u16(),
                retry_after: header_str(response.headers(), reqwest::header::RETRY_AFTER),
            });
        }
        if want_range > 0 && status != StatusCode::PARTIAL_CONTENT {
            // Server ignored Range: restart from zero, never append blindly.
            // The full body in hand IS the fresh start: truncate and stream it.
            match std::fs::remove_file(&part) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io_err("remove", &part, e)),
            }
            resumed = 0;
        }
        if want_range == 0 && status == StatusCode::PARTIAL_CONTENT {
            return Err(DownloadError::Http {
                status: status.as_u16(),
                retry_after: None,
            });
        }
        // HTML is rejected before hashing (spec §7): header here, bytes at verify.
        if is_html_response(response.headers()) {
            return Err(remove_part_quiet(&part, DownloadError::HtmlRejected));
        }
        let total = req.expected_size.or_else(|| {
            response
                .headers()
                .get(CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .map(|n| n + resumed)
        });
        let content_type = header_str(response.headers(), reqwest::header::CONTENT_TYPE);
        // Stream: append (resume) or create, hashing while writing, one pass.
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(resumed > 0)
            .write(true)
            .truncate(resumed == 0)
            .open(&part)
            .map_err(|e| io_err("open", &part, e))?;
        let mut sha1 = sha1::Sha1::new();
        let mut sha512 = sha2::Sha512::new();
        if resumed > 0 {
            // Pre-hash existing bytes or the resumed digest is wrong by construction.
            let existing: Vec<u8> = std::fs::read(&part).map_err(|e| io_err("read", &part, e))?;
            sha1.update(&existing);
            sha512.update(&existing);
        }
        let mut done = resumed;
        while let Some(bytes) = response.chunk().await.map_err(transport)? {
            while is_paused() {
                tokio::time::sleep(std::time::Duration::from_millis(PAUSE_POLL_MS)).await;
                if on_progress(DownloadProgress {
                    phase: DownloadPhase::Downloading,
                    done,
                    total,
                })
                .is_break()
                {
                    drop(file);
                    return Ok(DownloadOutcome {
                        status: DownloadStatus::Cancelled,
                        path: part.clone(),
                    });
                }
            }
            use std::io::Write;
            file.write_all(&bytes)
                .map_err(|e| io_err("write", &part, e))?;
            sha1.update(&bytes);
            sha512.update(&bytes);
            done += bytes.len() as u64;
            if let Some(e) = req.expected_size {
                if done > e {
                    drop(file);
                    return Err(remove_part_quiet(&part, DownloadError::SizeMismatch));
                }
            }
            if on_progress(DownloadProgress {
                phase: DownloadPhase::Downloading,
                done,
                total,
            })
            .is_break()
            {
                drop(file);
                return Ok(DownloadOutcome {
                    status: DownloadStatus::Cancelled,
                    path: part.clone(),
                });
            }
        }
        drop(file);
        // Re-hash from disk would double I/O; the stream hashers above already
        // cover exactly the landed bytes — verify consumes them via check.
        self.verify_streamed(req, content_type, done, sha1, sha512, on_progress)
            .await
    }

    /// Verify streamed bytes (size, HTML, hashes) and commit.
    async fn verify_streamed(
        &self,
        req: &DownloadRequest,
        content_type: Option<String>,
        done: u64,
        sha1: sha1::Sha1,
        sha512: sha2::Sha512,
        on_progress: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
    ) -> Result<DownloadOutcome, DownloadError> {
        use sha1::Digest as _;
        let part = atomic::part_path(&req.dest);
        let total = req.expected_size;
        if on_progress(DownloadProgress {
            phase: DownloadPhase::Verifying,
            done,
            total,
        })
        .is_break()
        {
            return Ok(DownloadOutcome {
                status: DownloadStatus::Cancelled,
                path: part.clone(),
            });
        }
        if let Some(e) = req.expected_size {
            if done != e {
                return Err(remove_part_quiet(&part, DownloadError::SizeMismatch));
            }
        }
        if let Some(ct) = content_type {
            if is_html_content_type(&ct) {
                return Err(remove_part_quiet(&part, DownloadError::HtmlRejected));
            }
        }
        // Byte sniff on the landed head: chunking-agnostic (spec §9: 1024 B).
        let head: Vec<u8> = tokio::task::spawn_blocking({
            let part = part.clone();
            move || read_head(&part)
        })
        .await
        .map_err(|_| DownloadError::Internal("hash task"))?
        .map_err(|e| io_err("read", &part, e))?;
        if is_html_bytes(&head) {
            return Err(remove_part_quiet(&part, DownloadError::HtmlRejected));
        }
        if let Some(want) = &req.sha1_hex {
            if hex_of(sha1.finalize()) != want.to_lowercase() {
                return Err(remove_part_quiet(&part, DownloadError::HashMismatch));
            }
        }
        if let Some(want) = &req.sha512_hex {
            if hex_of(sha512.finalize()) != want.to_lowercase() {
                return Err(remove_part_quiet(&part, DownloadError::HashMismatch));
            }
        }
        if on_progress(DownloadProgress {
            phase: DownloadPhase::Committing,
            done,
            total,
        })
        .is_break()
        {
            return Ok(DownloadOutcome {
                status: DownloadStatus::Cancelled,
                path: part.clone(),
            });
        }
        atomic::commit_part(&part, &req.dest)?;
        Ok(DownloadOutcome {
            status: DownloadStatus::Success,
            path: req.dest.clone(),
        })
    }

    /// Verify an existing complete `.part` (resume-skip path) or dest file.
    async fn verify_and_commit(
        &self,
        req: &DownloadRequest,
        content_type: Option<String>,
        on_progress: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
    ) -> Result<DownloadOutcome, DownloadError> {
        let part = atomic::part_path(&req.dest);
        let (len, sha1hex, sha512hex, head) = tokio::task::spawn_blocking({
            let part = part.clone();
            move || hash_file(&part)
        })
        .await
        .map_err(|_| DownloadError::Internal("hash task"))?
        .map_err(|e| io_err("read", &part, e))?;
        if on_progress(DownloadProgress {
            phase: DownloadPhase::Verifying,
            done: len,
            total: req.expected_size,
        })
        .is_break()
        {
            return Ok(DownloadOutcome {
                status: DownloadStatus::Cancelled,
                path: part.clone(),
            });
        }
        if let Some(e) = req.expected_size {
            if len != e {
                return Err(remove_part_quiet(&part, DownloadError::SizeMismatch));
            }
        }
        if let Some(ct) = content_type {
            if is_html_content_type(&ct) {
                return Err(remove_part_quiet(&part, DownloadError::HtmlRejected));
            }
        }
        if is_html_bytes(&head) {
            return Err(remove_part_quiet(&part, DownloadError::HtmlRejected));
        }
        if let Some(want) = &req.sha1_hex {
            if sha1hex != want.to_lowercase() {
                return Err(remove_part_quiet(&part, DownloadError::HashMismatch));
            }
        }
        if let Some(want) = &req.sha512_hex {
            if sha512hex != want.to_lowercase() {
                return Err(remove_part_quiet(&part, DownloadError::HashMismatch));
            }
        }
        if on_progress(DownloadProgress {
            phase: DownloadPhase::Committing,
            done: len,
            total: req.expected_size,
        })
        .is_break()
        {
            return Ok(DownloadOutcome {
                status: DownloadStatus::Cancelled,
                path: part.clone(),
            });
        }
        atomic::commit_part(&part, &req.dest)?;
        Ok(DownloadOutcome {
            status: DownloadStatus::Success,
            path: req.dest.clone(),
        })
    }
}

/// Hash a file in a blocking thread: returns (len, sha1hex, sha512hex, head).
fn hash_file(part: &Path) -> std::io::Result<(u64, String, String, Vec<u8>)> {
    let mut f = std::fs::File::open(part)?;
    let mut sha1 = sha1::Sha1::new();
    let mut sha512 = sha2::Sha512::new();
    let mut buf = [0u8; 65536];
    let mut len = 0u64;
    let mut head = Vec::new();
    loop {
        use std::io::Read;
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        sha1.update(&buf[..n]);
        sha512.update(&buf[..n]);
        if head.len() < HTML_SNIFF_LEN {
            head.extend_from_slice(&buf[..n.min(HTML_SNIFF_LEN - head.len())]);
        }
        len += n as u64;
    }
    Ok((
        len,
        hex_of(sha1.finalize()),
        hex_of(sha512.finalize()),
        head,
    ))
}

/// First bytes of a file for the HTML sniff.
fn read_head(part: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut f = std::fs::File::open(part)?;
    let mut head = vec![0u8; HTML_SNIFF_LEN];
    let mut got = 0;
    while got < HTML_SNIFF_LEN {
        let n = f.read(&mut head[got..])?;
        if n == 0 {
            break;
        }
        got += n;
    }
    head.truncate(got);
    Ok(head)
}

/// Pure existence + content check for the F-032 skip path (never deletes).
async fn check_file(path: &Path, req: &DownloadRequest) -> Result<(), DownloadError> {
    let want_len = req.expected_size;
    let want_sha1 = req.sha1_hex.clone();
    let want_sha512 = req.sha512_hex.clone();
    let owned = path.to_path_buf();
    let (len, sha1hex, sha512hex, _) = tokio::task::spawn_blocking(move || hash_file(&owned))
        .await
        .map_err(|_| DownloadError::Internal("hash task"))?
        .map_err(|e| io_err("read", path, e))?;
    if let Some(e) = want_len {
        if len != e {
            return Err(DownloadError::SizeMismatch);
        }
    }
    if let Some(want) = want_sha1 {
        if sha1hex != want.to_lowercase() {
            return Err(DownloadError::HashMismatch);
        }
    }
    if let Some(want) = want_sha512 {
        if sha512hex != want.to_lowercase() {
            return Err(DownloadError::HashMismatch);
        }
    }
    Ok(())
}

/// First free sibling: `artifact.jar` → `artifact-1.jar` (F-033).
fn unique_path(dest: &Path) -> PathBuf {
    let stem = dest
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "downloaded-mod".to_string());
    let ext = dest
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let parent = dest.parent().unwrap_or(Path::new("."));
    let mut n = 1u32;
    loop {
        let cand = parent.join(format!("{stem}-{n}{ext}"));
        if !cand.exists() && !atomic::part_path(&cand).exists() {
            return cand;
        }
        n += 1;
    }
}

fn real_free_space(path: &Path) -> std::io::Result<u64> {
    // Nearest existing ancestor: the dest itself may not exist yet.
    let mut probe = path;
    while !probe.exists() {
        match probe.parent() {
            Some(p) if p != probe => probe = p,
            _ => break,
        }
    }
    fs2::available_space(probe)
}

fn validate_url(raw: &str, allow_loopback: bool) -> Result<reqwest::Url, DownloadError> {
    let u = reqwest::Url::parse(raw).map_err(|_| DownloadError::InvalidUrl(raw.to_string()))?;
    if !u.username().is_empty() || u.password().is_some() {
        return Err(DownloadError::InvalidUrl(raw.to_string()));
    }
    let host = u
        .host_str()
        .ok_or_else(|| DownloadError::InvalidUrl(raw.to_string()))?;
    match u.scheme() {
        "https" => Ok(u),
        "http" if allow_loopback && is_loopback(host) => Ok(u),
        _ => Err(DownloadError::InvalidUrl(raw.to_string())),
    }
}

fn is_loopback(host: &str) -> bool {
    host == "localhost" || host.starts_with("127.") || host == "::1" || host == "[::1]"
}

fn transport(e: reqwest::Error) -> DownloadError {
    DownloadError::Transport(e.to_string())
}

fn header_str(h: &reqwest::header::HeaderMap, k: reqwest::header::HeaderName) -> Option<String> {
    h.get(k)?.to_str().ok().map(|s| s.to_string())
}

fn is_html_response(h: &reqwest::header::HeaderMap) -> bool {
    use reqwest::header::CONTENT_TYPE;
    h.get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(is_html_content_type)
        .unwrap_or(false)
}

fn is_html_content_type(ct: &str) -> bool {
    let mime = ct.split(';').next().unwrap_or("").trim().to_lowercase();
    mime == "text/html" || mime == "application/xhtml+xml"
}

fn is_html_bytes(first: &[u8]) -> bool {
    // Strip BOM + NUL + whitespace, lowercase, check the 4 spec prefixes.
    let mut s = first;
    loop {
        match s.split_first() {
            Some((0xef, r1)) if r1.starts_with(b"\xbb\xbf") => {
                s = &r1[2..];
            }
            Some((0x00 | b' ' | b'\t' | b'\r' | b'\n', rest)) => s = rest,
            _ => break,
        }
    }
    let head = String::from_utf8_lossy(&s[..s.len().min(64)]).to_lowercase();
    head.starts_with("<!doctype html")
        || head.starts_with("<html")
        || head.starts_with("<head")
        || head.starts_with("<body")
}

fn hex_of<D: AsRef<[u8]>>(d: D) -> String {
    d.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn io_err(op: &'static str, path: &Path, source: std::io::Error) -> DownloadError {
    DownloadError::Io {
        op,
        path: path.to_path_buf(),
        source,
    }
}

/// Delete the `.part` if present, but always report the ORIGINAL error:
/// a locked-file cleanup failure must not mask the verification failure.
fn remove_part_quiet(part: &Path, original: DownloadError) -> DownloadError {
    let _ = std::fs::remove_file(part);
    original
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn setup(server: &MockServer, file: &str) -> (tempfile::TempDir, DownloadRequest) {
        let dir = crate::test_tempdir();
        let req = DownloadRequest {
            url: format!("{}/{}", server.uri(), file),
            dest: dir.path().join(file),
            expected_size: None,
            sha1_hex: None,
            sha512_hex: None,
            allow_insecure_loopback: true,
        };
        (dir, req)
    }

    fn go(_p: DownloadProgress) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }

    fn still() -> bool {
        false
    }

    fn sha1hex(data: &[u8]) -> String {
        use sha1::Digest as _;
        let mut h = sha1::Sha1::new();
        h.update(data);
        hex_of(h.finalize())
    }

    #[tokio::test]
    async fn spec_f021_download_commits_verified() {
        let server = MockServer::start().await;
        let body = b"hello-mods";
        Mock::given(method("GET"))
            .and(path("/a.jar"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(&body[..]))
            .mount(&server)
            .await;
        let dl = Downloader::new(2).expect("client");
        let (_dir, mut r) = setup(&server, "a.jar");
        r.expected_size = Some(body.len() as u64);
        r.sha1_hex = Some(sha1hex(body));
        let out = dl.download(&r, &mut go, &still).await.expect("downloads");
        assert_eq!(out.status, DownloadStatus::Success);
        assert_eq!(std::fs::read(&out.path).expect("read"), body);
        assert!(!crate::safety::atomic::part_path(&out.path).exists());
        let received = server.received_requests().await.expect("count");
        assert_eq!(received.len(), 1);
        let ua = received[0]
            .headers
            .get("user-agent")
            .expect("user-agent header");
        assert_eq!(ua.to_str().expect("str"), crate::USER_AGENT);
    }

    #[tokio::test]
    async fn spec_f034_hash_mismatch_deletes_part() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/b.jar"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"corrupt!"))
            .mount(&server)
            .await;
        let dl = Downloader::new(2).expect("client");
        let (_dir, mut r) = setup(&server, "b.jar");
        r.sha1_hex = Some("da39a3ee5e6b4b0d3255bfef95601890afd80709".to_string());
        let dest = r.dest.clone();
        assert!(matches!(
            dl.download(&r, &mut go, &still).await,
            Err(DownloadError::HashMismatch)
        ));
        assert!(!dest.exists());
        assert!(!crate::safety::atomic::part_path(&dest).exists());
    }

    #[tokio::test]
    async fn spec_f035_cancel_keeps_part() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/c.jar"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"0123456789"))
            .mount(&server)
            .await;
        let dl = Downloader::new(2).expect("client");
        let (_dir, r) = setup(&server, "c.jar");
        let dest = r.dest.clone();
        let mut cb = |p: DownloadProgress| {
            if p.done > 0 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        };
        let out = dl
            .download(&r, &mut cb, &still)
            .await
            .expect("cancel is status");
        assert_eq!(out.status, DownloadStatus::Cancelled);
        assert_eq!(out.path, crate::safety::atomic::part_path(&dest));
        assert!(!dest.exists());
        assert!(crate::safety::atomic::part_path(&dest).exists());
    }

    #[tokio::test]
    async fn spec_f027_no_retry_on_404() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/gone.jar"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let dl = Downloader::new(2).expect("client");
        let (_dir, r) = setup(&server, "gone.jar");
        assert!(matches!(
            dl.download(&r, &mut go, &still).await,
            Err(DownloadError::Http { status: 404, .. })
        ));
        assert_eq!(server.received_requests().await.expect("count").len(), 1);
    }

    #[tokio::test]
    async fn spec_f026_resume_appends_on_206() {
        let server = MockServer::start().await;
        let full = b"0123456789";
        Mock::given(method("GET"))
            .and(path("/d.jar"))
            .and(header("Range", "bytes=5-"))
            .respond_with(ResponseTemplate::new(206).set_body_bytes(&full[5..]))
            .mount(&server)
            .await;
        let dl = Downloader::new(2).expect("client");
        let (_dir, r) = setup(&server, "d.jar");
        std::fs::write(crate::safety::atomic::part_path(&r.dest), &full[..5])
            .expect("pre-seed part");
        let out = dl.download(&r, &mut go, &still).await.expect("resumes");
        assert_eq!(out.status, DownloadStatus::Success);
        assert_eq!(std::fs::read(&out.path).expect("read"), full);
    }

    #[tokio::test]
    async fn spec_f026_restart_when_server_ignores_range() {
        let server = MockServer::start().await;
        let full = b"abcdef";
        Mock::given(method("GET"))
            .and(path("/e.jar"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(&full[..]))
            .mount(&server)
            .await;
        let dl = Downloader::new(2).expect("client");
        let (_dir, r) = setup(&server, "e.jar");
        std::fs::write(crate::safety::atomic::part_path(&r.dest), b"STALE").expect("pre-seed part");
        let out = dl.download(&r, &mut go, &still).await.expect("restarts");
        assert_eq!(out.status, DownloadStatus::Success);
        assert_eq!(std::fs::read(&out.path).expect("read"), full);
    }

    #[tokio::test]
    async fn spec_f027_retry_after_honored() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/f.jar"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/f.jar"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"ok"))
            .mount(&server)
            .await;
        let policy = retry::RetryPolicy {
            max_attempts: 2,
            ..Default::default()
        };
        let dl = Downloader::with_policy(2, policy).expect("client");
        let (_dir, r) = setup(&server, "f.jar");
        let t0 = std::time::Instant::now();
        let out = dl.download(&r, &mut go, &still).await.expect("retries");
        assert_eq!(out.status, DownloadStatus::Success);
        assert!(t0.elapsed() >= std::time::Duration::from_secs(1));
        assert_eq!(server.received_requests().await.expect("count").len(), 2);
    }

    #[tokio::test]
    async fn spec_f031_redirect_limit_reached() {
        let server = MockServer::start().await;
        for i in 0..11 {
            Mock::given(method("GET"))
                .and(path(format!("/r{i}")))
                .respond_with(
                    ResponseTemplate::new(302).insert_header("Location", format!("/r{}", i + 1)),
                )
                .mount(&server)
                .await;
        }
        let dl = Downloader::new(2).expect("client");
        let (_dir, r) = setup(&server, "r0");
        assert!(matches!(
            dl.download(&r, &mut go, &still).await,
            Err(DownloadError::RedirectLimit)
        ));
    }

    #[tokio::test]
    async fn spec_f023_disk_preflight_blocks() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/g.jar"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"data"))
            .mount(&server)
            .await;
        let mut dl = Downloader::new(2).expect("client");
        dl.set_space_probe(|_| Ok(1));
        let (_dir, mut r) = setup(&server, "g.jar");
        r.expected_size = Some(1_000_000);
        assert!(matches!(
            dl.download(&r, &mut go, &still).await,
            Err(DownloadError::NoSpace)
        ));
        assert_eq!(server.received_requests().await.expect("count").len(), 0);
    }

    #[tokio::test]
    async fn spec_f030_html_rejected_deletes_part() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/h.html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "text/html")
                    .set_body_bytes(b"<html>nope</html>"),
            )
            .mount(&server)
            .await;
        let dl = Downloader::new(2).expect("client");
        let (_dir, r) = setup(&server, "h.html");
        let dest = r.dest.clone();
        assert!(matches!(
            dl.download(&r, &mut go, &still).await,
            Err(DownloadError::HtmlRejected)
        ));
        assert!(!dest.exists());
        assert!(!crate::safety::atomic::part_path(&dest).exists());
    }

    #[tokio::test]
    async fn spec_f031_rejects_non_https() {
        let dl = Downloader::new(2).expect("client");
        let dir = crate::test_tempdir();
        let r = DownloadRequest {
            url: "http://example.com/x.jar".to_string(),
            dest: dir.path().join("x.jar"),
            allow_insecure_loopback: false,
            ..Default::default()
        };
        assert!(matches!(
            dl.download(&r, &mut go, &still).await,
            Err(DownloadError::InvalidUrl(_))
        ));
    }

    #[tokio::test]
    async fn spec_f032_skips_verified_existing() {
        let server = MockServer::start().await;
        let body = b"already-here";
        let dl = Downloader::new(2).expect("client");
        let dir = crate::test_tempdir();
        let dest = dir.path().join("s.jar");
        std::fs::write(&dest, body).expect("pre-seed dest");
        let r = DownloadRequest {
            url: format!("{}/s.jar", server.uri()),
            dest: dest.clone(),
            expected_size: Some(body.len() as u64),
            sha1_hex: Some(sha1hex(body)),
            sha512_hex: None,
            allow_insecure_loopback: true,
        };
        let out = dl.download(&r, &mut go, &still).await.expect("skips");
        assert_eq!(out.status, DownloadStatus::Skipped);
        assert_eq!(out.path, dest);
        assert_eq!(server.received_requests().await.expect("count").len(), 0);
    }

    #[tokio::test]
    async fn spec_f033_unique_name_on_mismatch() {
        let server = MockServer::start().await;
        let body = b"correct-bytes";
        Mock::given(method("GET"))
            .and(path("/u.jar"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(&body[..]))
            .mount(&server)
            .await;
        let dl = Downloader::new(2).expect("client");
        let dir = crate::test_tempdir();
        let dest = dir.path().join("u.jar");
        std::fs::write(&dest, b"stale-user-bytes").expect("pre-seed dest");
        let r = DownloadRequest {
            url: format!("{}/u.jar", server.uri()),
            dest: dest.clone(),
            expected_size: Some(body.len() as u64),
            sha1_hex: Some(sha1hex(body)),
            sha512_hex: None,
            allow_insecure_loopback: true,
        };
        let out = dl
            .download(&r, &mut go, &still)
            .await
            .expect("downloads aside");
        assert_eq!(out.status, DownloadStatus::Success);
        assert_eq!(out.path, dir.path().join("u-1.jar"));
        assert_eq!(std::fs::read(&out.path).expect("read"), body);
        assert_eq!(
            std::fs::read(&dest).expect("read"),
            b"stale-user-bytes",
            "user file untouched"
        );
    }

    #[test]
    fn spec_f039_concurrency_clamped_1_to_8() {
        let big = Downloader::new(100).expect("client");
        assert_eq!(big.sem.available_permits(), 8);
        let none = Downloader::new(0).expect("client");
        assert_eq!(none.sem.available_permits(), 1);
    }

    #[tokio::test]
    async fn spec_f035_pause_polls_then_proceeds() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/p.jar"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"paused-ok"))
            .mount(&server)
            .await;
        let dl = Downloader::new(2).expect("client");
        let (_dir, r) = setup(&server, "p.jar");
        static POLLS: AtomicUsize = AtomicUsize::new(0);
        let paused = || POLLS.fetch_add(1, Ordering::SeqCst) < 2;
        let out = dl
            .download(&r, &mut go, &paused)
            .await
            .expect("proceeds after pause");
        assert_eq!(out.status, DownloadStatus::Success);
        assert!(POLLS.load(Ordering::SeqCst) >= 2);
    }
}
