//! Updates: QuipKeys asks GitHub for the latest release, and on request
//! downloads it, checks its SHA-256 against the release's SHA256SUMS file,
//! puts it in place of the running copy and restarts.
//!
//! The check sends nothing about the user: it is a plain request for the
//! public release list. It happens only when the user chooses "Check for
//! updates", or daily if they switched on automatic checks (off by default).
//!
//! Releases are built by .github/workflows/release.yml when a version tag
//! ("v0.2.0") is pushed. Assets: the Windows program, the macOS app (zipped)
//! and SHA256SUMS.

use serde_json::Value;

pub const REPO: &str = "codinghasnat/quipkeys";
pub const WINDOWS_ASSET: &str = "quipkeys-windows-x64.exe";
pub const MAC_ASSET: &str = "QuipKeys-macos.zip";

/// A release newer than this one.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    /// The release's web page.
    pub page: String,
    pub asset_url: String,
    pub asset_name: String,
    pub sums_url: Option<String>,
}

/// This copy's version.
pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// "v1.2.3" or "1.2.3" -> (1, 2, 3).
pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches('v');
    let mut it = s.split(['.', '-', '+']).map(|p| p.parse::<u32>());
    let major = it.next()?.ok()?;
    let minor = it.next().and_then(|r| r.ok()).unwrap_or(0);
    let patch = it.next().and_then(|r| r.ok()).unwrap_or(0);
    Some((major, minor, patch))
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// The asset this platform installs.
pub fn asset_name() -> &'static str {
    if cfg!(target_os = "macos") {
        MAC_ASSET
    } else {
        WINDOWS_ASSET
    }
}

/// The release described by GitHub's "latest release" JSON, if it's newer
/// than `current` and has the asset this platform needs.
pub fn release_from_json(v: &Value, current: &str, asset: &str) -> Option<Release> {
    if v.get("draft").and_then(Value::as_bool) == Some(true) || v.get("prerelease").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let tag = v.get("tag_name")?.as_str()?;
    if !is_newer(tag, current) {
        return None;
    }
    let assets = v.get("assets")?.as_array()?;
    let url_of = |name: &str| {
        assets
            .iter()
            .find(|a| a.get("name").and_then(Value::as_str) == Some(name))
            .and_then(|a| a.get("browser_download_url").and_then(Value::as_str))
            .map(String::from)
    };
    Some(Release {
        version: tag.trim_start_matches('v').to_string(),
        page: v.get("html_url").and_then(Value::as_str).unwrap_or("").to_string(),
        asset_url: url_of(asset)?,
        asset_name: asset.to_string(),
        sums_url: url_of("SHA256SUMS"),
    })
}

/// The checksum listed for `name` in a SHA256SUMS file ("<hex>  <name>").
pub fn expected_sha(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let hash = it.next()?;
        let file = it.next()?.trim_start_matches('*');
        (file == name && hash.len() == 64).then(|| hash.to_lowercase())
    })
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Is there a newer release? Ok(None) if this copy is the latest.
pub fn check() -> Result<Option<Release>, String> {
    let resp = crate::ai::http()
        .get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .set("user-agent", "QuipKeys-updater")
        .set("accept", "application/vnd.github+json")
        .call();
    match resp {
        Ok(r) => {
            let v: Value = r.into_json().map_err(|_| "Couldn't read the release list.".to_string())?;
            Ok(release_from_json(&v, current(), asset_name()))
        }
        // No release published yet.
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(ureq::Error::Status(code, _)) => Err(format!("GitHub answered with error {code}. Try again later.")),
        Err(_) => Err("Couldn't reach GitHub to check for updates. Check your internet connection.".into()),
    }
}

/// Download the release asset and check it against SHA256SUMS.
pub fn download_verified(rel: &Release) -> Result<Vec<u8>, String> {
    let sums_url = rel.sums_url.as_ref().ok_or("This release has no checksums, so it can't be installed safely.")?;
    let get = |url: &str| -> Result<Vec<u8>, String> {
        let r = crate::ai::http().get(url).set("user-agent", "QuipKeys-updater").call().map_err(|_| "The download failed. Try again later.".to_string())?;
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut std::io::Read::take(r.into_reader(), 300 << 20), &mut buf).map_err(|_| "The download was interrupted.".to_string())?;
        Ok(buf)
    };
    let sums = String::from_utf8_lossy(&get(sums_url)?).into_owned();
    let want = expected_sha(&sums, &rel.asset_name).ok_or("The release's checksum file doesn't list this download.")?;
    let bytes = get(&rel.asset_url)?;
    if sha256_hex(&bytes) != want {
        return Err("The download didn't match its checksum, so it wasn't installed.".into());
    }
    Ok(bytes)
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
/// Put `bytes` where `target` (a running program) is: the old file is
/// renamed out of the way first (Windows lets a running program be renamed
/// but not overwritten), and put back if the new one can't be written.
pub fn swap_file(target: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let old = target.with_extension("old.exe");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(target, &old).map_err(|_| "QuipKeys can't replace itself in this folder. Download the new version from the release page instead.".to_string())?;
    if let Err(_e) = std::fs::write(target, bytes) {
        let _ = std::fs::rename(&old, target);
        return Err("Couldn't write the new version. Download it from the release page instead.".into());
    }
    Ok(())
}

/// Remove what an update left behind (the old program on Windows).
pub fn clean_up() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old.exe"));
    }
}

/// Download, verify, install and restart. Only returns on failure.
#[cfg(windows)]
pub fn install(rel: &Release) -> Result<(), String> {
    let bytes = download_verified(rel)?;
    let exe = std::env::current_exe().map_err(|_| "Couldn't find QuipKeys' own program file.".to_string())?;
    swap_file(&exe, &bytes)?;
    crate::stats::save();
    let _ = std::process::Command::new(&exe).arg("--restarted").spawn();
    std::process::exit(0);
}

/// Download, verify, install and restart. Only returns on failure.
#[cfg(target_os = "macos")]
pub fn install(rel: &Release) -> Result<(), String> {
    use std::process::Command;
    let bytes = download_verified(rel)?;
    let exe = std::env::current_exe().map_err(|_| "Couldn't find the QuipKeys app.".to_string())?;
    let bundle = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).ok_or("QuipKeys isn't running from an app bundle.")?.to_path_buf();
    let parent = bundle.parent().ok_or("Couldn't find the folder QuipKeys is in.")?;
    // Stage next to the app, so the final moves stay on one disk.
    let stage = parent.join(".QuipKeys-update");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|_| "QuipKeys can't write to its folder. Move it to Applications, or download the update from the release page.".to_string())?;
    let zip = stage.join(MAC_ASSET);
    std::fs::write(&zip, &bytes).map_err(|_| "Couldn't save the download.".to_string())?;
    let ok = Command::new("/usr/bin/ditto").args(["-x", "-k"]).arg(&zip).arg(&stage).status().map(|s| s.success()).unwrap_or(false);
    let new_app = stage.join("QuipKeys.app");
    if !ok || !new_app.exists() {
        let _ = std::fs::remove_dir_all(&stage);
        return Err("Couldn't unpack the update.".into());
    }
    let old_app = stage.join("QuipKeys-old.app");
    std::fs::rename(&bundle, &old_app).map_err(|_| "Couldn't move the old version aside.".to_string())?;
    if std::fs::rename(&new_app, &bundle).is_err() {
        let _ = std::fs::rename(&old_app, &bundle);
        return Err("Couldn't put the new version in place.".into());
    }
    crate::stats::save();
    // A new ad-hoc signed build is a new app to macOS: its Accessibility
    // switch must be turned on again (the welcome window walks through it).
    let _ = Command::new("/usr/bin/tccutil").args(["reset", "Accessibility", "com.quipkeys.app"]).status();
    let _ = Command::new("/usr/bin/open").arg("-n").arg(&bundle).args(["--args", "--restarted"]).spawn();
    let _ = std::fs::remove_dir_all(&stage);
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions_compare() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("0.10"), Some((0, 10, 0)));
        assert!(is_newer("v0.2.0", "0.1.9") && is_newer("v0.10.0", "0.9.9") && is_newer("1.0.0", "0.99.0"));
        assert!(!is_newer("v0.1.0", "0.1.0") && !is_newer("v0.0.9", "0.1.0") && !is_newer("nightly", "0.1.0"));
    }

    fn release(tag: &str, assets: &[&str]) -> Value {
        json!({
            "tag_name": tag, "html_url": "https://github.com/x/releases/tag/v0.2.0", "draft": false, "prerelease": false,
            "assets": assets.iter().map(|n| json!({"name": n, "browser_download_url": format!("https://dl/{n}")})).collect::<Vec<_>>(),
        })
    }

    #[test]
    fn picks_this_platforms_download() {
        let v = release("v0.2.0", &[WINDOWS_ASSET, MAC_ASSET, "SHA256SUMS"]);
        let r = release_from_json(&v, "0.1.0", WINDOWS_ASSET).unwrap();
        assert_eq!((r.version.as_str(), r.asset_url.as_str()), ("0.2.0", "https://dl/quipkeys-windows-x64.exe"));
        assert_eq!(r.sums_url.as_deref(), Some("https://dl/SHA256SUMS"));
        assert_eq!(release_from_json(&v, "0.2.0", WINDOWS_ASSET), None, "not newer");
        assert_eq!(release_from_json(&release("v0.3.0", &[MAC_ASSET]), "0.1.0", WINDOWS_ASSET), None, "no Windows build");
        let mut pre = release("v0.4.0", &[WINDOWS_ASSET]);
        pre["prerelease"] = json!(true);
        assert_eq!(release_from_json(&pre, "0.1.0", WINDOWS_ASSET), None, "pre-releases are skipped");
    }

    #[test]
    fn checksums() {
        let h = sha256_hex(b"abc");
        assert_eq!(h, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let sums = format!("{h}  quipkeys-windows-x64.exe\n{}  QuipKeys-macos.zip\n", "0".repeat(64));
        assert_eq!(expected_sha(&sums, WINDOWS_ASSET).as_deref(), Some(h.as_str()));
        assert_eq!(expected_sha(&sums, "other.zip"), None);
    }

    #[test]
    fn swaps_a_file_in_place() {
        let dir = std::env::temp_dir().join(format!("qk-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("quipkeys.exe");
        std::fs::write(&exe, b"old").unwrap();
        swap_file(&exe, b"new").unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert_eq!(std::fs::read(dir.join("quipkeys.old.exe")).unwrap(), b"old");
        // A second update replaces the leftover from the first.
        swap_file(&exe, b"newer").unwrap();
        assert_eq!(std::fs::read(dir.join("quipkeys.old.exe")).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
