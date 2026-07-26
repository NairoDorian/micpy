use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

const SCRCPY_DIR: &str = "scrcpy";
const GITHUB_API: &str = "https://api.github.com/repos/Genymobile/scrcpy/releases/latest";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ManagedScrcpyStatus {
    pub ready: bool,
    pub version: String,
    pub path: String,
    pub is_managed: bool,
}

fn portable_dir() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join(SCRCPY_DIR))
}

fn managed_dir() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("micpy").join(SCRCPY_DIR))
}

fn scrcpy_exe_in(dir: &PathBuf) -> PathBuf {
    dir.join("scrcpy.exe")
}

fn get_scrcpy_version(path: &std::path::Path) -> Option<String> {
    let mut cmd = Command::new(path);
    crate::configure_command(&mut cmd);
    let output = cmd.arg("--version").output().ok()?;
    if !output.status.success() { return None; }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().next().and_then(|l| {
        l.split_whitespace().nth(1).map(|v| v.to_string())
    })
}

fn get_scrcpy_version_at(path: &std::path::Path) -> Option<String> {
    if !path.exists() { return None; }
    get_scrcpy_version(path)
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

fn fetch_latest_release() -> Result<GithubRelease, String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(format!("micpy/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let resp = client
        .get(GITHUB_API)
        .send()
        .map_err(|e| format!("GitHub API request failed: {}", e))?
        .error_for_status()
        .map_err(|e| format!("GitHub API error: {}", e))?;

    resp.json().map_err(|e| format!("Failed to parse GitHub release JSON: {}", e))
}

fn find_win64_asset(release: &GithubRelease) -> Option<&GithubAsset> {
    release.assets.iter().find(|a| a.name.contains("win64") && a.name.ends_with(".zip"))
}

fn parse_version(tag: &str) -> (u32, u32, u32) {
    let mut it = tag.trim_start_matches('v')
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

pub fn find_scrcpy() -> ManagedScrcpyStatus {
    if let Some(dir) = portable_dir() {
        let exe_path = scrcpy_exe_in(&dir);
        if let Some(ver) = get_scrcpy_version_at(&exe_path) {
            return ManagedScrcpyStatus { ready: true, version: ver, path: exe_path.to_string_lossy().to_string(), is_managed: true };
        }
    }

    if let Some(dir) = managed_dir() {
        let exe_path = scrcpy_exe_in(&dir);
        if let Some(ver) = get_scrcpy_version_at(&exe_path) {
            return ManagedScrcpyStatus { ready: true, version: ver, path: exe_path.to_string_lossy().to_string(), is_managed: true };
        }
    }

    match get_scrcpy_version(std::path::Path::new("scrcpy")) {
        Some(ver) => ManagedScrcpyStatus { ready: true, version: ver, path: "scrcpy".to_string(), is_managed: false },
        None => ManagedScrcpyStatus { ready: false, version: String::new(), path: String::new(), is_managed: false },
    }
}

pub fn get_target_dir() -> Option<PathBuf> {
    portable_dir().filter(|d| crate::utils::is_writable(d)).or_else(managed_dir)
}

pub fn ensure_scrcpy() -> Result<ManagedScrcpyStatus, String> {
    let current = find_scrcpy();

    // If we already have a managed copy, check the latest GitHub release.
    if current.ready && current.is_managed {
        let latest = fetch_latest_release().ok();
        if let Some(release) = latest {
            let curr_ver = parse_version(&current.version);
            let latest_ver = parse_version(&release.tag_name);
            if curr_ver >= latest_ver {
                return Ok(current);
            }
        } else {
            return Ok(current);
        }
    }

    let release = fetch_latest_release()?;
    let asset = find_win64_asset(&release).ok_or_else(|| format!("No Windows 64-bit zip found in release {}", release.tag_name))?;
        let target_dir = get_target_dir().ok_or("Failed to determine target directory")?;

    let exe_path = scrcpy_exe_in(&target_dir);
    if exe_path.exists() {
        crate::utils::remove_all(&target_dir)?;
    }

    std::fs::create_dir_all(&target_dir).map_err(|e| format!("Failed to create directory {:?}: {}", target_dir, e))?;

    let zip_name = &asset.name;
    let staging_dir = std::env::temp_dir().join("micpy");
    std::fs::create_dir_all(&staging_dir).map_err(|e| format!("Failed to create staging directory {:?}: {}", staging_dir, e))?;
    let zip_path = staging_dir.join(zip_name);

    crate::utils::download_file(&asset.browser_download_url, &zip_path)?;
    crate::utils::extract_zip(&zip_path, &target_dir, 1)?;
    std::fs::remove_file(&zip_path).map_err(|e| format!("Failed to remove staging zip {:?}: {}", zip_path, e))?;

    Ok(find_scrcpy())
}
