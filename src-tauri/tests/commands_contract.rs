//! Contract tests for the IPC layer.
//!
//! These check the wire shape and the error keys, because a mismatch between
//! Rust and the frontend cannot be caught by either compiler: TypeScript
//! believes what the hand-written types in ipc.ts claim, and Rust never sees
//! them. A rename on one side only shows up as a runtime failure otherwise.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Command names registered in the handler must match those the frontend calls.
#[test]
fn every_invoked_command_is_registered() {
    let lib_rs = read(&repo_root().join("src-tauri").join("src").join("lib.rs"));
    let ipc_ts = read(&repo_root().join("src").join("lib").join("ipc.ts"));

    // Handlers are listed as `module::name,` and live in several modules, so
    // match on the path shape rather than on one module's name.
    let registered: BTreeSet<String> = lib_rs
        .lines()
        .map(|l| l.trim().trim_end_matches(','))
        .filter_map(|l| l.rsplit_once("::"))
        .map(|(_, name)| name.to_string())
        .filter(|name| {
            !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_')
        })
        .collect();

    let mut invoked = BTreeSet::new();
    // Matches invoke<...>("name") and invoke("name").
    for (idx, _) in ipc_ts.match_indices("invoke") {
        let rest = &ipc_ts[idx..];
        let Some(open) = rest.find('(') else { continue };
        let after = &rest[open + 1..];
        let trimmed = after.trim_start();
        let Some(quote) = trimmed.chars().next() else { continue };
        if quote != '"' && quote != '\'' {
            continue;
        }
        let body = &trimmed[1..];
        if let Some(end) = body.find(quote) {
            invoked.insert(body[..end].to_string());
        }
    }

    assert!(!registered.is_empty(), "no commands found in lib.rs");
    assert!(!invoked.is_empty(), "no invoke() calls found in ipc.ts");

    let missing: Vec<_> = invoked.difference(&registered).collect();
    assert!(
        missing.is_empty(),
        "frontend calls commands that are not registered in lib.rs: {missing:?}"
    );
}

/// Commands taking a whole photo must accept raw bytes, not a JSON array.
///
/// A 2000x1333 image is 10.7MB, which as a JSON number array becomes 32MB of
/// text costing around two seconds to encode and parse. Anything reintroducing
/// `rgba: Vec<u8>` as a command argument would silently make the app slow
/// again, which is easy to do and hard to notice.
#[test]
fn pixel_commands_take_a_raw_body() {
    let src = repo_root().join("src-tauri").join("src");
    let sources: String = std::fs::read_dir(&src)
        .expect("cannot read src-tauri/src")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .map(|p| read(&p))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        !sources.contains("rgba: Vec<u8>"),
        "a command takes pixels as a JSON array; use tauri::ipc::Request and raw_body instead"
    );

    let ipc_ts = read(&repo_root().join("src").join("lib").join("ipc.ts"));
    assert!(
        !ipc_ts.contains("Array.from(rgba)") && !ipc_ts.contains("Array.from(photo.rgba)"),
        "ipc.ts converts pixels to a number array; send the ArrayBuffer instead"
    );
}

/// The binary layouts are defined twice — once in Rust, once in TypeScript —
/// and nothing but agreement makes them work. These pin the field order so a
/// change on one side without the other fails here rather than at print time.
#[test]
fn binary_protocol_layouts_match_on_both_sides() {
    let background_rs = read(&repo_root().join("src-tauri").join("src").join("background.rs"));
    let commands_rs = read(&repo_root().join("src-tauri").join("src").join("commands.rs"));
    let ipc_ts = read(&repo_root().join("src").join("lib").join("ipc.ts"));

    // Mask response: width, height, ratio, name length, name, data.
    for field in ["mask.width.to_le_bytes", "mask.height.to_le_bytes", "ratio.to_le_bytes"] {
        assert!(background_rs.contains(field), "mask_response no longer writes {field}");
    }
    for read_call in ["getUint32(0, true)", "getUint32(4, true)", "getFloat32(8, true)"] {
        assert!(ipc_ts.contains(read_call), "decodeMask no longer reads {read_call}");
    }

    // Print body: a little-endian u32 JSON length, then the JSON, then pixels.
    assert!(
        commands_rs.contains("u32::from_le_bytes([body[0], body[1], body[2], body[3]])"),
        "parse_print_request no longer reads a little-endian length prefix"
    );
    assert!(
        ipc_ts.contains("setUint32(0, json.byteLength, true)"),
        "invokePrint no longer writes a little-endian length prefix"
    );
}

/// Error keys produced by Rust must exist in the translation table, or the UI
/// would display a raw key like "error.print.failed" to the user.
#[test]
fn every_error_key_has_a_translation() {
    let src = repo_root().join("src-tauri").join("src");
    // Read every module in the directory rather than a hand-kept list: a new
    // command module would otherwise escape this check silently, which is the
    // exact failure the test exists to prevent.
    let sources: String = std::fs::read_dir(&src)
        .expect("cannot read src-tauri/src")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .map(|p| read(&p))
        .collect::<Vec<_>>()
        .join("\n");
    let i18n_ts = read(&repo_root().join("src").join("lib").join("i18n.ts"));

    let mut keys = BTreeSet::new();
    for (idx, _) in sources.match_indices("\"error.") {
        let rest = &sources[idx + 1..];
        if let Some(end) = rest.find('"') {
            keys.insert(rest[..end].to_string());
        }
    }

    assert!(!keys.is_empty(), "no error keys found in the command modules");

    let missing: Vec<_> = keys
        .iter()
        .filter(|k| !i18n_ts.contains(&format!("\"{k}\"")))
        .collect();
    assert!(
        missing.is_empty(),
        "error keys with no Croatian translation: {missing:?}"
    );
}

/// Every message the rules engine can emit must have a Croatian translation.
///
/// The keys live in the domain crate rather than in a command module, so the
/// error-key test above does not see them; without this a failed rule would
/// render in the panel as the bare key.
#[test]
fn every_rule_message_has_a_translation() {
    let rules_rs = read(
        &repo_root()
            .join("crates")
            .join("domain")
            .join("src")
            .join("rules.rs"),
    );
    let i18n_ts = read(&repo_root().join("src").join("lib").join("i18n.ts"));

    let mut keys = BTreeSet::new();
    for (idx, _) in rules_rs.match_indices("\"rule.") {
        let rest = &rules_rs[idx + 1..];
        let Some(end) = rest.find('"') else { continue };
        let key = &rest[..end];
        // Skip prefixes used for `starts_with` assertions in the tests, which
        // are not keys anyone looks up.
        if key.ends_with('.') {
            continue;
        }
        keys.insert(key.to_string());
    }

    assert!(!keys.is_empty(), "no rule message keys found");
    let missing: Vec<_> = keys
        .iter()
        .filter(|k| !i18n_ts.contains(&format!("\"{k}\"")))
        .collect();
    assert!(missing.is_empty(), "rule keys with no translation: {missing:?}");
}

/// Placeholders in a translation must be supplied by the code that raises it.
/// A message reading "Ispis nije uspio. ({detail})" with no detail passed would
/// show the literal braces.
#[test]
fn error_translations_have_no_unfilled_placeholders() {
    let commands_rs = read(&repo_root().join("src-tauri").join("src").join("commands.rs"));
    let i18n_ts = read(&repo_root().join("src").join("lib").join("i18n.ts"));

    // Keys raised with json!({}) supply nothing, so their message must have no
    // placeholders.
    for line in commands_rs.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("UiError::new(\"") else {
            continue;
        };
        let Some(end) = rest.find('"') else { continue };
        let key = &rest[..end];

        // Find the translation for this key.
        let needle = format!("\"{key}\":");
        let Some(pos) = i18n_ts.find(&needle) else { continue };
        let after = &i18n_ts[pos + needle.len()..];
        let Some(line_end) = after.find(",\n") else { continue };
        let message = &after[..line_end];

        assert!(
            !message.contains('{'),
            "key {key} is raised without params but its message has placeholders: {message}"
        );
    }
}

/// The offline guarantee must be declared in the Tauri config, not merely
/// intended. This is the verifiable half of "no network calls".
///
/// `connect-src` cannot be `'none'`: Tauri sends a raw request body through
/// `fetch` to the IPC endpoint, and blocking it makes the webview fall back to
/// `postMessage`, which carries JSON only — so every photo arrives as a number
/// array and the pixel commands reject it. The two IPC origins are local to the
/// application; what matters for the guarantee is that no outside origin is
/// reachable.
#[test]
fn tauri_config_forbids_outbound_connections() {
    let conf = read(&repo_root().join("src-tauri").join("tauri.conf.json"));
    let compact: String = conf.chars().filter(|c| !c.is_whitespace()).collect();

    assert!(compact.contains("\"csp\":"), "no CSP declared");
    assert!(
        compact.contains("default-src'self'"),
        "CSP must set default-src 'self'"
    );

    let csp = conf
        .lines()
        .find(|l| l.contains("\"csp\""))
        .expect("no CSP line");
    let connect = csp
        .split(';')
        .find(|d| d.trim_start_matches(|c: char| !c.is_alphabetic()).starts_with("connect-src"))
        .expect("CSP must declare connect-src rather than inherit default-src");

    // Only the local IPC endpoints; anything else would be a way out.
    for source in connect.split_whitespace().skip(1) {
        let source = source.trim_end_matches(['"', ';']);
        assert!(
            matches!(source, "ipc:" | "http://ipc.localhost" | "'self'"),
            "connect-src allows {source}, which is not a local IPC origin"
        );
    }
}

/// The bundle must not enable an updater, which would phone home on startup.
#[test]
fn no_updater_configured() {
    let conf = read(&repo_root().join("src-tauri").join("tauri.conf.json"));
    let compact: String = conf.chars().filter(|c| !c.is_whitespace()).collect();

    assert!(
        !compact.contains("\"updater\""),
        "an updater is configured; it would contact a server on startup"
    );
    assert!(
        !compact.contains("\"createUpdaterArtifacts\":true"),
        "updater artifacts are enabled"
    );
}
