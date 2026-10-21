//! CLI command implementations: clap-thin wiring over mm-core.

pub mod install;
pub mod list;
pub mod plan;
pub mod remove;
pub mod scan;
pub mod search;
pub mod update;

pub use install::install;
pub use list::list;
pub use plan::plan;
pub use remove::remove;
pub use scan::scan;
pub use search::search;
pub use update::update;

use mm_core::providers::ModrinthClient;
use mm_core::resolver::{
    requires_confirmation, resolve_and_plan, BatchPlan, PlanEntry, PlanSummary, Warning,
};
use std::collections::HashSet;
use std::path::PathBuf;

/// Shared context: paths come from flags, never defaults.
#[derive(Debug, Clone, Default)]
pub struct Ctx {
    pub instance: Option<PathBuf>,
    pub api_base: Option<String>,
    /// The operator's explicit opt-in for plain HTTP to loopback
    /// (`--insecure-loopback`). Never inferred from a URL.
    pub insecure_loopback: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("usage: {0}")]
    Usage(String),
    #[error("provider: {0}")]
    Provider(String),
    #[error("download: {0}")]
    Download(String),
    #[error("io: {0}")]
    Io(String),
    #[error("blocked: {0}")]
    Blocked(String),
    #[error("confirmation required: pass --yes")]
    ConfirmationRequired,
}

impl CliError {
    /// Exit codes: 0 ok · 1 operational error · 2 usage / confirmation.
    pub fn code(&self) -> i32 {
        match self {
            CliError::Usage(_) | CliError::ConfirmationRequired => 2,
            _ => 1,
        }
    }
}

pub fn info() -> Result<(), CliError> {
    let info = mm_core::app_info();
    println!(
        "{} {} ({} / {})",
        info.name, info.version, info.os, info.arch
    );
    Ok(())
}

fn io_err(op: &str, e: std::io::Error) -> CliError {
    CliError::Io(format!("{op}: {e}"))
}

pub fn client(ctx: &Ctx) -> Result<ModrinthClient, CliError> {
    match ctx.api_base.as_deref() {
        Some(base) => ModrinthClient::with_base(base),
        None => ModrinthClient::new(),
    }
    .map_err(|e| CliError::Provider(e.to_string()))
}

pub fn instance_dir(ctx: &Ctx) -> Result<PathBuf, CliError> {
    ctx.instance
        .clone()
        .ok_or_else(|| CliError::Usage("missing --instance <DIR>".to_string()))
}

pub fn mods_dir(ctx: &Ctx) -> Result<PathBuf, CliError> {
    Ok(instance_dir(ctx)?.join("mods"))
}

pub fn lock_path(ctx: &Ctx) -> Result<PathBuf, CliError> {
    Ok(instance_dir(ctx)?.join("instance.lock.json"))
}

/// Builds the `DownloadRequest` for every CLI download, one place, so the
/// transport decision cannot drift between commands.
///
/// The transport rule is ours, not the remote's: https only, exactly like
/// the Tauri app (`commands.rs` passes `false` there). Plain HTTP is enabled
/// by one thing only — the operator passing the hidden `--insecure-loopback`
/// flag, which states that this run was started against a local test server.
/// A provider response is data, and data does not choose the transport:
/// `allow_insecure(&file.url)` used to make a response naming
/// `http://127.evil.com` — a DNS name anyone can register — a plain-HTTP
/// fetch with the remote author instead of us choosing the transport
/// (AGENTS.md §5.11, `User-Agent` and CDN trust boundaries).
///
/// Core still re-checks every hop, so even with the flag only loopback
/// *addresses* are admitted and `127.evil.com` stays refused.
pub(crate) fn build_download_request(
    ctx: &Ctx,
    url: String,
    dest: PathBuf,
    expected_size: Option<u64>,
    sha1_hex: Option<String>,
    sha512_hex: Option<String>,
) -> mm_core::downloader::DownloadRequest {
    mm_core::downloader::DownloadRequest {
        url,
        dest,
        expected_size,
        sha1_hex,
        sha512_hex,
        // The operator's explicit flag, and nothing else. It used to be
        // derived from `api_base`, which meant a response naming
        // `http://127.evil.com` - a DNS name anyone can register - could
        // choose the transport. A provider response is data, and data does
        // not choose the transport.
        allow_insecure_loopback: ctx.insecure_loopback,
    }
}

pub fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    println!("{}", headers.join(" | "));
    for r in rows {
        println!("{}", r.join(" | "));
    }
}

/// Plan-first gate: `--yes` bypasses; a TTY prompts; anything else refuses.
pub fn confirm(summary: &PlanSummary, yes: bool) -> Result<(), CliError> {
    if !requires_confirmation(summary) || yes {
        return Ok(());
    }
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return Err(CliError::ConfirmationRequired);
    }
    eprint!(
        "Apply {} change(s) ({} blocked)? [y/N] ",
        summary.total_count, summary.blocked_count
    );
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| io_err("read stdin", e))?;
    let ok = matches!(line.trim().to_lowercase().as_str(), "y" | "yes");
    if ok {
        Ok(())
    } else {
        Err(CliError::ConfirmationRequired)
    }
}

/// Unconditional gate for update/remove: any row needs confirmation.
pub fn confirm_forced(count: usize, yes: bool) -> Result<(), CliError> {
    if count == 0 || yes {
        return Ok(());
    }
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return Err(CliError::ConfirmationRequired);
    }
    eprint!("Apply {count} change(s)? [y/N] ");
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| io_err("read stdin", e))?;
    if matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(CliError::ConfirmationRequired)
    }
}
/// Resolve slugs through the shared core facade (one implementation for
/// CLI and Tauri).
pub async fn resolve_slugs(
    provider: &dyn mm_core::providers::Provider,
    slugs: &[String],
    game: &str,
    loader: &str,
    existing: &HashSet<String>,
) -> Result<BatchPlan, CliError> {
    if slugs.is_empty() {
        return Err(CliError::Usage("no projects given".to_string()));
    }
    resolve_and_plan(provider, slugs, game, loader, existing)
        .await
        .map_err(|e| CliError::Provider(e.to_string()))
}

pub fn print_plan(entries: &[PlanEntry], summary: &PlanSummary, warnings: &[Warning]) {
    print_table(
        &["status", "source", "title", "version", "file", "reason"],
        &entries
            .iter()
            .map(|e| {
                vec![
                    e.status.clone(),
                    e.source.clone(),
                    e.title.clone(),
                    e.version_number.clone().unwrap_or_default(),
                    e.filename.clone().unwrap_or_default(),
                    e.reason.clone(),
                ]
            })
            .collect::<Vec<_>>(),
    );
    println!(
        "selected={} dependencies={} downloadable={} blocked={}",
        summary.selected_count,
        summary.dependency_count,
        summary.downloadable_count,
        summary.blocked_count
    );
    for w in warnings {
        println!("[{}] {}", w.severity, w.message);
    }
}

#[cfg(test)]
mod tests {
    use super::{build_download_request, Ctx};
    use std::path::PathBuf;

    fn ctx(api_base: Option<&str>, insecure_loopback: bool) -> Ctx {
        Ctx {
            instance: None,
            api_base: api_base.map(str::to_string),
            insecure_loopback,
        }
    }

    fn request_for(ctx: &Ctx, url: &str) -> mm_core::downloader::DownloadRequest {
        build_download_request(
            ctx,
            url.to_string(),
            PathBuf::from("x.jar"),
            None,
            None,
            None,
        )
    }

    /// The CLI transport decision against old code: `allow_insecure(&file.url)`
    /// read the flag out of the provider's URL, and the old `is_loopback`
    /// prefix check made `127.evil.com` count as loopback. Remote data must
    /// never weaken the transport rule.
    #[test]
    fn spec_f031_cli_never_enables_insecure_http_from_remote_urls() {
        let production = ctx(None, false);
        for url in [
            "http://127.evil.com/x.jar",
            "http://127.0.0.1.evil.io/x.jar",
            "http://127.0.0.1/x.jar",
            "http://localhost/x.jar",
        ] {
            let req = request_for(&production, url);
            assert!(
                !req.allow_insecure_loopback,
                "{url} must not weaken the transport rule"
            );
        }
    }

    /// `--api-base` is a routing option, not a transport decision. A loopback
    /// base used to switch plain HTTP on as a side effect; it must not any
    /// more. The move from `--api-base` alone is exactly the regression the
    /// next assertion pins.
    #[test]
    fn spec_f031_cli_insecure_http_is_not_derived_from_api_base() {
        let loopback_api = ctx(Some("http://127.0.0.1:8080/v2"), false);
        for url in [
            "http://127.0.0.1:8080/x.jar",
            "https://api.modrinth.com/x.jar",
        ] {
            assert!(
                !request_for(&loopback_api, url).allow_insecure_loopback,
                "{url}: --api-base must not enable plain HTTP"
            );
        }

        // The flag is the switch, and it is the whole switch.
        let flagged = ctx(None, true);
        assert!(request_for(&flagged, "http://127.0.0.1:8080/x.jar").allow_insecure_loopback);
    }

    /// The rule through the real downloader: no flag → even a live plain-HTTP
    /// loopback URL is refused (and never fetched); flag → the same URL lands.
    #[tokio::test]
    async fn spec_f031_cli_plain_http_loopback_needs_the_flag() {
        use mm_core::downloader::client::DownloadError;
        use mm_core::downloader::{DownloadProgress, Downloader};
        use std::ops::ControlFlow;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/x.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"payload"))
            .mount(&server)
            .await;
        let url = format!("{}/x.bin", server.uri());
        let dir = tempfile::tempdir().expect("temp dir");
        let dl = Downloader::new(1).expect("client");
        let mut go = |_p: DownloadProgress| ControlFlow::Continue(());
        let still = || false;

        // The api-base points at the same loopback server: still refused.
        let strict = ctx(Some(&url), false);
        let req = build_download_request(
            &strict,
            url.clone(),
            dir.path().join("refused.bin"),
            None,
            None,
            None,
        );
        assert!(matches!(
            dl.download(&req, &mut go, &still).await,
            Err(DownloadError::InvalidUrl(_))
        ));
        assert_eq!(
            server.received_requests().await.expect("count").len(),
            0,
            "a refused scheme never reaches the socket"
        );

        let flagged = ctx(None, true);
        let req = build_download_request(
            &flagged,
            url,
            dir.path().join("allowed.bin"),
            None,
            None,
            None,
        );
        let out = dl
            .download(&req, &mut go, &still)
            .await
            .expect("flag allows loopback http");
        assert_eq!(std::fs::read(&out.path).expect("read"), b"payload");
    }
}
