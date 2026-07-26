use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

const ADB_DIR: &str = "adb";
const PLATFORM_TOOLS_URL: &str = "https://dl.google.com/android/repository/platform-tools-latest-windows.zip";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AdbStatus {
    pub available: bool,
    pub path: String,
    pub version: String,
    pub is_managed: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AdbDevice {
    pub serial: String,
    pub state: String,
    pub model: String,
}

fn portable_dir() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join(ADB_DIR))
}

fn managed_dir() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("micpy").join(ADB_DIR))
}

fn adb_exe_in(dir: &PathBuf) -> PathBuf {
    if cfg!(target_os = "windows") {
        dir.join("adb.exe")
    } else {
        dir.join("adb")
    }
}

pub fn find_adb() -> AdbStatus {
    if let Some(dir) = portable_dir() {
        let exe_path = adb_exe_in(&dir);
        if exe_path.exists() {
            let ver = get_version(&exe_path);
            return AdbStatus { available: true, version: ver, path: exe_path.to_string_lossy().to_string(), is_managed: true };
        }
    }

    if let Some(dir) = managed_dir() {
        let exe_path = adb_exe_in(&dir);
        if exe_path.exists() {
            let ver = get_version(&exe_path);
            return AdbStatus { available: true, version: ver, path: exe_path.to_string_lossy().to_string(), is_managed: true };
        }
    }

    let mut cmd = Command::new("adb");
    crate::configure_command(&mut cmd);
    match cmd.arg("--version").output() {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let ver = stdout.lines().next().unwrap_or("adb").to_string();
            AdbStatus { available: true, version: ver, path: "adb".to_string(), is_managed: false }
        }
        _ => AdbStatus { available: false, version: String::new(), path: String::new(), is_managed: false },
    }
}

fn get_version(path: &PathBuf) -> String {
    // Use configure_command to suppress the console window that would flash
    // when running adb --version from a GUI application on Windows.
    let mut cmd = Command::new(path);
    crate::configure_command(&mut cmd);
    if let Ok(output) = cmd.arg("--version").output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            return stdout.lines().next().unwrap_or("adb").to_string();
        }
    }
    String::new()
}

pub fn get_target_dir() -> Option<PathBuf> {
    portable_dir().or_else(managed_dir)
}

pub fn get_adb_path() -> Option<std::path::PathBuf> {
    portable_dir()
        .and_then(|dir| { let exe = adb_exe_in(&dir); if exe.exists() { Some(exe) } else { None } })
        .or_else(|| managed_dir().and_then(|dir| { let exe = adb_exe_in(&dir); if exe.exists() { Some(exe) } else { None } }))
        .or_else(|| {
            let mut cmd = Command::new("adb");
            crate::configure_command(&mut cmd);
            cmd.arg("--version").output().ok()?;
            Some(std::path::PathBuf::from("adb"))
        })
}

pub fn ensure_adb() -> Result<AdbStatus, String> {
    let current = find_adb();

    if current.available {
        return Ok(current);
    }

    let target_dir = get_target_dir().ok_or("Failed to determine target directory")?;

    let exe_path = adb_exe_in(&target_dir);
    if exe_path.exists() {
        crate::utils::remove_all(&target_dir)?;
    }

    std::fs::create_dir_all(&target_dir).map_err(|e| format!("Failed to create directory {:?}: {}", target_dir, e))?;

    let zip_name = "platform-tools-latest-windows.zip";
    let zip_path = target_dir.parent().unwrap_or(&target_dir).join(zip_name);

    crate::utils::download_file(PLATFORM_TOOLS_URL, &zip_path)?;
    crate::utils::extract_zip(&zip_path, &target_dir, 1)?;
    crate::utils::remove_all(&zip_path)?;

    Ok(find_adb())
}

pub fn list_devices(adb_path: Option<&str>) -> Result<Vec<AdbDevice>, String> {
    let adb_bin = adb_path.unwrap_or("adb");

    let output = {
        let mut cmd = Command::new(adb_bin);
        crate::configure_command(&mut cmd);
        cmd.arg("devices").arg("-l").output()
    }.map_err(|e| format!("Failed to execute adb: {}", e))?;

    if !output.status.success() {
        return Err(format!("adb devices failed: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut devices = Vec::new();

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("List of devices") || trimmed.starts_with('*') {
            continue;
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() >= 2 {
            let serial = parts[0].to_string();
            let state = parts[1].to_string();
            let mut model = "Android Device".to_string();

            for part in &parts[2..] {
                if part.starts_with("model:") {
                    model = part.trim_start_matches("model:").replace('_', " ");
                }
            }

            devices.push(AdbDevice { serial, state, model });
        }
    }

    Ok(devices)
}

pub fn connect_wireless(ip_port: &str, adb_path: Option<&str>) -> Result<String, String> {
    let adb_bin = adb_path.unwrap_or("adb");

    let output = {
        let mut cmd = Command::new(adb_bin);
        crate::configure_command(&mut cmd);
        cmd.arg("connect").arg(ip_port).output()
    }.map_err(|e| format!("Failed to execute adb connect: {}", e))?;

    let result = String::from_utf8_lossy(&output.stdout).to_string();
    if output.status.success() {
        Ok(result.trim().to_string())
    } else {
        Err(result.trim().to_string())
    }
}

pub fn get_last_wireless_device() -> Option<String> {
    let config_path = get_target_dir()?.join("last_wireless_device.txt");
    std::fs::read_to_string(config_path).ok()
}

pub fn save_last_wireless_device(device: &str) -> Result<(), String> {
    let config_path = get_target_dir().ok_or("No target directory")?.join("last_wireless_device.txt");
    std::fs::write(config_path, device).map_err(|e| format!("Failed to save device: {}", e))
}