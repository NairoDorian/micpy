use std::ffi::OsStr;
#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, PathBuf};

pub fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

pub fn download_file(url: &str, dest: &PathBuf) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(format!("micpy/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("Download request failed: {}", e))?
        .error_for_status()
        .map_err(|e| format!("Download failed: {}", e))?;

    let bytes = response
        .bytes()
        .map_err(|e| format!("Failed to read download stream: {}", e))?;

    std::fs::write(dest, &bytes).map_err(|e| format!("Failed to write to {:?}: {}", dest, e))?;

    Ok(())
}

// Extract a zip archive. strip_prefix=N skips the first N path components in each entry,
// which is needed because Platform Tools zips contain a "platform-tools/" root folder
// that we don't want replicated in our target directory.
pub fn extract_zip(zip_path: &PathBuf, target_dir: &PathBuf, strip_prefix: usize) -> Result<(), String> {
    let file =
        std::fs::File::open(zip_path).map_err(|e| format!("Failed to open zip file: {}", e))?;

    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Failed to read zip archive: {}", e))?;

    std::fs::create_dir_all(target_dir)
        .map_err(|e| format!("Failed to create target directory {:?}: {}", target_dir, e))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read zip entry {}: {}", i, e))?;

        let safe = entry.enclosed_name().ok_or_else(|| format!("zip entry {} has an unsafe path", i))?;
        let stripped: PathBuf = safe.components().skip(strip_prefix).collect();
        if stripped.as_os_str().is_empty() { continue; }
        if stripped.components().any(|c| !matches!(c, Component::Normal(_))) {
            return Err(format!("zip entry {} escapes the target directory", i));
        }
        let out_path = target_dir.join(&stripped);

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| format!("Failed to create dir {:?}: {}", out_path, e))?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create parent {:?}: {}", parent, e))?;
            }
            let mut outfile = std::fs::File::create(&out_path)
                .map_err(|e| format!("Failed to create file {:?}: {}", out_path, e))?;
            std::io::copy(&mut entry, &mut outfile)
                .map_err(|e| format!("Failed to extract {:?}: {}", out_path, e))?;
        }
    }

    Ok(())
}

pub fn remove_all(path: &PathBuf) -> Result<(), String> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
            .map_err(|e| format!("Failed to remove directory {:?}: {}", path, e))?;
    } else if path.exists() {
        std::fs::remove_file(path)
            .map_err(|e| format!("Failed to remove file {:?}: {}", path, e))?;
    }
    Ok(())
}

/// Check whether a directory exists and is writable by attempting to
/// create a probe file inside it and immediately deleting it.
pub fn is_writable(dir: &PathBuf) -> bool {
    let probe = dir.join(".micpy-write-test");
    std::fs::create_dir_all(dir).is_ok()
        && std::fs::write(&probe, b"").is_ok()
        && {
            let _ = std::fs::remove_file(&probe);
            true
        }
}
