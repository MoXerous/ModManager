//! CLI end-to-end: wiremock serves Modrinth API + CDN bytes into a temp
//! instance. No live network, no real mod folders.

use std::io::Write;
use std::path::{Path, PathBuf};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_modmanager"))
}

/// The wiremock CDN serves plain `http` on 127.0.0.1, so every CLI run here
/// names the loopback seam explicitly. Nothing else enables it: a loopback
/// `--api-base` alone must not (see `e2e_install_refuses_plain_http_without_flag`).
const INSECURE_LOOPBACK: &str = "--insecure-loopback";

fn run(args: &[&str]) -> (i32, String, String) {
    let out = std::process::Command::new(bin())
        .args(args)
        .output()
        .expect("spawn cli");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn tmp_instance() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/test-tmp");
    let _ = std::fs::create_dir_all(&base);
    tempfile::Builder::new()
        .prefix("mm-cli-")
        .tempdir_in(&base)
        .unwrap_or_else(|_| tempfile::tempdir().expect("tmp"))
}

/// A real zip (downloader rejects non-archives) + its real SHA-1/SHA-512.
fn fixture_jar(mod_id: &str) -> (Vec<u8>, String, String) {
    let body = format!(
        r#"{{"id": "{mod_id}", "name": "{mod_id}", "version": "1.0",
            "description": "d", "depends": {{"minecraft": "1.21"}}}}"#
    );
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        w.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default())
            .expect("entry");
        w.write_all(body.as_bytes()).expect("write");
        w.finish().expect("finish");
    }
    let bytes = buf.into_inner();
    use sha1::Digest as Digest1;
    use sha2::Digest as Digest512;
    let mut h1 = sha1::Sha1::new();
    Digest1::update(&mut h1, &bytes);
    let mut h2 = sha2::Sha512::new();
    Digest512::update(&mut h2, &bytes);
    (
        bytes,
        hex::encode(h1.finalize()),
        hex::encode(h2.finalize()),
    )
}

/// Minimal hex without a new dep.
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
    }
}

struct Api {
    server: MockServer,
    sha1: String,
}

impl Api {
    async fn setup() -> Self {
        let server = MockServer::start().await;
        let (jar, sha1, sha512) = fixture_jar("testmod");
        let cdn = format!("{}/cdn/x.jar", server.uri());
        let search = serde_json::json!({
            "hits": [{
                "project_id": "PTEST", "project_type": "mod", "slug": "testmod",
                "author": "t", "title": "Test Mod", "description": "d",
                "categories": ["fabric"], "display_categories": ["fabric"],
                "versions": ["1.21"], "downloads": 7, "follows": 1,
                "icon_url": null, "date_created": "2024-01-01T00:00:00Z",
                "date_modified": "2024-01-01T00:00:00Z", "latest_version": "V1",
                "license": "MIT", "client_side": "required", "server_side": "optional",
                "gallery": [], "featured_gallery": null, "color": null
            }],
            "offset": 0, "limit": 10, "total_hits": 1
        });
        let project = serde_json::json!({
            "id": "PTEST", "slug": "testmod", "title": "Test Mod",
            "description": "d", "body": "b", "client_side": "required",
            "server_side": "optional", "game_versions": ["1.21"],
            "project_type": "mod", "downloads": 7, "followers": 1,
            "categories": ["fabric"], "loaders": ["fabric"], "versions": ["V1"],
            "icon_url": null, "team": "t", "status": "approved",
            "license": {"id": "MIT"}, "gallery": []
        });
        let versions = serde_json::json!([{
            "id": "V1", "project_id": "PTEST", "author_id": "a",
            "featured": false, "name": "1.0", "version_number": "1.0.0",
            "changelog": null, "date_published": "2024-01-01T00:00:00Z",
            "downloads": 1, "version_type": "release", "status": "listed",
            "game_versions": ["1.21"], "loaders": ["fabric"],
            "files": [{
                "hashes": {"sha512": &sha512, "sha1": &sha1},
                "url": &cdn, "filename": "x.jar",
                "primary": true, "size": jar.len(), "file_type": null
            }],
            "dependencies": []
        }]);
        Mock::given(method("GET"))
            .and(path("/v2/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&search))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/project/testmod"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&project))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/project/PTEST/version"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&versions))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/cdn/x.jar"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(jar.clone()))
            .mount(&server)
            .await;
        Self { server, sha1 }
    }

    fn api_base(&self) -> String {
        self.server.uri()
    }
}

fn api_arg(api: &Api, dir: &tempfile::TempDir) -> (String, String) {
    (
        dir.path().to_string_lossy().into_owned(),
        api.api_base() + "/v2",
    )
}

#[tokio::test]
async fn e2e_search_lists_slug() {
    let api = Api::setup().await;
    let dir = tmp_instance();
    let (inst, base) = api_arg(&api, &dir);
    let (code, out, _) = run(&[
        "--instance",
        &inst,
        "--api-base",
        &base,
        INSECURE_LOOPBACK,
        "search",
        "test",
    ]);
    assert_eq!(code, 0);
    assert!(out.contains("testmod"), "{out}");
}

#[tokio::test]
async fn e2e_plan_writes_nothing() {
    let api = Api::setup().await;
    let dir = tmp_instance();
    let (inst, base) = api_arg(&api, &dir);
    let (code, out, _) = run(&[
        "--instance",
        &inst,
        "--api-base",
        &base,
        INSECURE_LOOPBACK,
        "plan",
        "testmod",
        "--game",
        "1.21",
        "--loader",
        "fabric",
    ]);
    assert_eq!(code, 0);
    assert!(out.contains("downloadable"), "{out}");
    assert!(!dir.path().join("instance.lock.json").exists());
    assert!(!dir.path().join("mods").exists());
}

#[tokio::test]
async fn e2e_install_then_list_then_remove() {
    let api = Api::setup().await;
    let dir = tmp_instance();
    let (inst, base) = api_arg(&api, &dir);
    let shared = [
        "--instance",
        inst.as_str(),
        "--api-base",
        base.as_str(),
        INSECURE_LOOPBACK,
    ];
    let mut install = shared.to_vec();
    install.extend([
        "install", "testmod", "--game", "1.21", "--loader", "fabric", "--yes",
    ]);
    let (code, _, err) = run(&install);
    assert_eq!(code, 0, "{err}");
    let jar = dir.path().join("mods").join("x.jar");
    assert!(jar.is_file());
    assert!(dir.path().join("mods").read_dir().expect("rd").all(|e| {
        !e.expect("e")
            .file_name()
            .to_string_lossy()
            .ends_with(".part")
    }));
    let lock = std::fs::read_to_string(dir.path().join("instance.lock.json")).expect("lock");
    assert!(lock.contains("PTEST"), "{lock}");

    let mut list = shared.to_vec();
    list.push("list");
    let (code, out, _) = run(&list);
    assert_eq!(code, 0);
    assert!(out.contains("x.jar"), "{out}");

    let mut scan = shared.to_vec();
    scan.push("scan");
    let (code, out, _) = run(&scan);
    assert_eq!(code, 0);
    assert!(out.contains("x.jar"), "{out}");

    let mut remove = shared.to_vec();
    remove.extend(["remove", "testmod", "--yes"]);
    let (code, _, err) = run(&remove);
    assert_eq!(code, 0, "{err}");
    assert!(!jar.exists());
    let lock = std::fs::read_to_string(dir.path().join("instance.lock.json")).expect("lock");
    assert!(!lock.contains("PTEST"), "{lock}");
    let backup = dir.path().join(".modmanager-removed");
    assert!(backup.is_dir());
}

/// The transport rule at the operator's boundary: pointing `--api-base` at a
/// plain-HTTP loopback server is a routing choice, not permission to fetch
/// over HTTP. Without the explicit flag the download is refused before a
/// single byte is requested.
#[tokio::test]
async fn e2e_install_refuses_plain_http_without_flag() {
    let api = Api::setup().await;
    let dir = tmp_instance();
    let (inst, base) = api_arg(&api, &dir);
    let (code, _, err) = run(&[
        "--instance",
        &inst,
        "--api-base",
        &base,
        "install",
        "testmod",
        "--game",
        "1.21",
        "--loader",
        "fabric",
        "--yes",
    ]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("invalid url"), "{err}");
    assert!(!dir.path().join("mods").join("x.jar").exists());
    assert!(!dir.path().join("instance.lock.json").exists());
    let hits = api.server.received_requests().await.expect("count");
    assert!(
        !hits.iter().any(|r| r.url.path() == "/cdn/x.jar"),
        "plain http must be refused before the CDN is contacted"
    );
}

#[tokio::test]
async fn e2e_update_bumps_version() {
    let api = Api::setup().await;
    let dir = tmp_instance();
    let (inst, base) = api_arg(&api, &dir);
    // Install v1 first.
    let (code, _, err) = run(&[
        "--instance",
        &inst,
        "--api-base",
        &base,
        INSECURE_LOOPBACK,
        "install",
        "testmod",
        "--game",
        "1.21",
        "--loader",
        "fabric",
        "--yes",
    ]);
    assert_eq!(code, 0, "{err}");
    // Serve v2 for the same sha1 via the update endpoint.
    let (jar2, sha2, sha512_2) = fixture_jar("testmod");
    let cdn2 = format!("{}/cdn/y.jar", api.server.uri());
    let v2 = serde_json::json!({
        api.sha1.clone(): {
            "id": "V2", "project_id": "PTEST", "author_id": "a",
            "featured": false, "name": "2.0", "version_number": "2.0.0",
            "changelog": null, "date_published": "2024-02-01T00:00:00Z",
            "downloads": 2, "version_type": "release", "status": "listed",
            "game_versions": ["1.21"], "loaders": ["fabric"],
            "files": [{
                "hashes": {"sha512": &sha512_2, "sha1": &sha2},
                "url": &cdn2, "filename": "y.jar",
                "primary": true, "size": jar2.len(), "file_type": null
            }],
            "dependencies": []
        }
    });
    Mock::given(method("POST"))
        .and(path("/v2/version_files/update"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&v2))
        .mount(&api.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/cdn/y.jar"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(jar2))
        .mount(&api.server)
        .await;
    let (code, out, err) = run(&[
        "--instance",
        &inst,
        "--api-base",
        &base,
        INSECURE_LOOPBACK,
        "update",
        "--game",
        "1.21",
        "--loader",
        "fabric",
        "--yes",
    ]);
    assert_eq!(code, 0, "{err} {out}");
    assert!(out.contains("2.0.0"), "{out}");
    let lock = std::fs::read_to_string(dir.path().join("instance.lock.json")).expect("lock");
    assert!(lock.contains("V2"), "{lock}");
    assert!(dir.path().join("mods").join("y.jar").is_file());
}

#[tokio::test]
async fn e2e_confirmation_required_without_yes() {
    // Test processes run without a TTY: remove (always gated) without
    // --yes exits 2 and changes nothing.
    let api = Api::setup().await;
    let dir = tmp_instance();
    let (inst, base) = api_arg(&api, &dir);
    let (code, _, err) = run(&[
        "--instance",
        &inst,
        "--api-base",
        &base,
        INSECURE_LOOPBACK,
        "install",
        "testmod",
        "--game",
        "1.21",
        "--loader",
        "fabric",
        "--yes",
    ]);
    assert_eq!(code, 0, "{err}");
    let (code, _, _) = run(&[
        "--instance",
        &inst,
        "--api-base",
        &base,
        INSECURE_LOOPBACK,
        "remove",
        "testmod",
    ]);
    assert_eq!(code, 2);
    assert!(dir.path().join("mods").join("x.jar").is_file());
    let lock = std::fs::read_to_string(dir.path().join("instance.lock.json")).expect("lock");
    assert!(lock.contains("PTEST"), "{lock}");
}

#[test]
fn e2e_info_smoke() {
    let (code, out, _) = run(&["info"]);
    assert_eq!(code, 0);
    assert!(out.contains("ModManager"), "{out}");
}
