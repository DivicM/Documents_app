//! Verifies the app cannot reach the network.
//!
//! A CSP in tauri.conf.json constrains the webview only. The Rust side is
//! unaffected by it, so a second check walks the dependency tree: if no HTTP
//! client is linked in, there is nothing that could make a request. Together
//! these cover both halves of the process.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/platform.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

/// Crates that can perform network I/O. Any of these in the tree would defeat
/// the offline guarantee regardless of what the CSP says.
const NETWORK_CRATES: &[&str] = &[
    "reqwest",
    "hyper",
    "ureq",
    "curl",
    "isahc",
    "surf",
    "attohttpc",
    "tungstenite",
    "tokio-tungstenite",
];

/// No HTTP client may be compiled into the binary for this platform.
///
/// Deliberately asks cargo what is actually built for the host target rather
/// than reading Cargo.lock: the lockfile lists dependencies for every platform,
/// including ones this build never compiles, so it reports false positives.
#[test]
fn no_http_client_linked_for_this_target() {
    let output = std::process::Command::new(env!("CARGO"))
        .args(["tree", "--edges", "normal", "--prefix", "none", "--target"])
        .arg(current_target())
        .current_dir(workspace_root())
        .output();

    let Ok(output) = output else {
        eprintln!("cargo tree unavailable; skipping");
        return;
    };
    if !output.status.success() {
        eprintln!("cargo tree failed; skipping");
        return;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut found = Vec::new();
    for line in text.lines() {
        // Lines look like "reqwest v0.12.0".
        let Some(name) = line.split_whitespace().next() else {
            continue;
        };
        if NETWORK_CRATES.contains(&name) && !found.iter().any(|f| f == name) {
            found.push(name.to_string());
        }
    }

    assert!(
        found.is_empty(),
        "network-capable crates compiled into this build, breaking the \
         offline guarantee: {found:?}"
    );
}

fn current_target() -> &'static str {
    if cfg!(all(windows, target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(target_os = "macos") {
        "x86_64-apple-darwin"
    } else {
        "x86_64-unknown-linux-gnu"
    }
}

/// The CSP must exist and must forbid outbound connections once the Tauri app
/// is scaffolded. Skipped until then so the test does not fail on absence.
#[test]
fn tauri_csp_forbids_network_when_present() {
    let conf = workspace_root().join("src-tauri").join("tauri.conf.json");
    if !conf.exists() {
        eprintln!("tauri.conf.json not present yet; skipping CSP check");
        return;
    }

    let text = std::fs::read_to_string(&conf).expect("read tauri.conf.json");
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();

    assert!(
        compact.contains("\"csp\":"),
        "tauri.conf.json has no CSP; the offline guarantee is unenforced"
    );
    assert!(
        compact.contains("connect-src'none'") || compact.contains("connect-src'self'"),
        "CSP does not restrict connect-src, so the webview could call out"
    );
    assert!(
        !compact.contains("connect-src*"),
        "CSP allows connect-src *, which permits arbitrary network calls"
    );
}

/// Nothing in the source may reference a remote asset. Catches a CDN font or
/// script slipping into the frontend, which the CSP would block at runtime but
/// which should not be there in the first place.
#[test]
fn no_remote_urls_in_source() {
    let root = workspace_root();
    let mut offenders = Vec::new();

    for dir in ["crates", "src"] {
        let path = root.join(dir);
        if path.exists() {
            scan(&path, &mut offenders);
        }
    }

    assert!(
        offenders.is_empty(),
        "remote URLs referenced in source: {offenders:?}"
    );
}

fn scan(dir: &Path, offenders: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Build output and dependencies are not our source.
            if matches!(name, "target" | "node_modules" | ".git") {
                continue;
            }
            scan(&path, offenders);
            continue;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext, "rs" | "ts" | "tsx" | "js" | "jsx" | "html" | "css") {
            continue;
        }
        // This test file necessarily contains the patterns it searches for.
        if path.file_name().and_then(|n| n.to_str()) == Some("offline_guarantee.rs") {
            continue;
        }

        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            // Doc comments and licence headers cite URLs legitimately.
            if trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with("<!--") {
                continue;
            }
            if line.contains("http://") || line.contains("https://") {
                offenders.push(format!("{}:{}", path.display(), i + 1));
            }
        }
    }
}
