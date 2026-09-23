use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
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

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct AdbDevice {
    pub serial: String,
    pub state: String,
    pub model: String,
    #[serde(default)]
    pub is_wireless: bool,
    #[serde(default)]
    pub alias: Option<String>,
}

fn portable_dir() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join(ADB_DIR))
}

fn managed_dir() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("micpy").join(ADB_DIR))
}

fn adb_exe_in(dir: &Path) -> PathBuf {
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

fn get_version(path: &Path) -> String {
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
    portable_dir().filter(crate::utils::is_writable).or_else(managed_dir)
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

    // No adb.exe exists in the target dir at this point (find_adb would have
    // returned it), so extract over it without wiping the settings files that
    // live alongside (device_aliases.json, last_wireless_device.txt).
    let target_dir = get_target_dir().ok_or("Failed to determine target directory")?;

    std::fs::create_dir_all(&target_dir).map_err(|e| format!("Failed to create directory {:?}: {}", target_dir, e))?;

    let zip_name = "platform-tools-latest-windows.zip";
    let staging_dir = std::env::temp_dir().join("micpy");
    std::fs::create_dir_all(&staging_dir).map_err(|e| format!("Failed to create staging directory {:?}: {}", staging_dir, e))?;
    let zip_path = staging_dir.join(zip_name);

    crate::utils::download_file(PLATFORM_TOOLS_URL, &zip_path)?;
    crate::utils::extract_zip(&zip_path, &target_dir, 1)?;
    std::fs::remove_file(&zip_path).map_err(|e| format!("Failed to remove staging zip {:?}: {}", zip_path, e))?;

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
    let aliases = get_device_aliases();

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

            let is_wireless = serial.contains(':');
            let alias = aliases.get(&serial).cloned();
            devices.push(AdbDevice { serial, state, model, is_wireless, alias });
        }
    }

    Ok(devices)
}

pub fn get_device_aliases() -> std::collections::HashMap<String, String> {
    if let Some(target) = get_target_dir() {
        let file = target.join("device_aliases.json");
        if let Ok(content) = std::fs::read_to_string(file) {
            if let Ok(map) = serde_json::from_str::<std::collections::HashMap<String, String>>(&content) {
                return map;
            }
        }
    }
    std::collections::HashMap::new()
}

pub fn save_device_alias(serial: &str, alias: &str) -> Result<(), String> {
    let file = config_dir()?.join("device_aliases.json");
    let mut map = get_device_aliases();
    if alias.trim().is_empty() {
        map.remove(serial);
    } else {
        map.insert(serial.to_string(), alias.trim().to_string());
    }
    let json = serde_json::to_string_pretty(&map).map_err(|e| format!("JSON error: {e}"))?;
    std::fs::write(file, json).map_err(|e| format!("Failed to write aliases file: {e}"))
}

pub fn list_audio_encoders(serial: Option<&str>, scrcpy_path: Option<&str>) -> Result<Vec<String>, String> {
    let scrcpy_status = crate::scrcpy_manager::find_scrcpy();
    let scrcpy_bin = scrcpy_path
        .filter(|p| !p.trim().is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            if scrcpy_status.ready {
                Some(std::path::PathBuf::from(scrcpy_status.path))
            } else {
                None
            }
        })
        .unwrap_or_else(|| std::path::PathBuf::from("scrcpy"));

    let mut cmd = Command::new(scrcpy_bin);
    crate::configure_command(&mut cmd);
    cmd.arg("--list-encoders");
    if let Some(s) = serial {
        if !s.trim().is_empty() {
            cmd.args(["-s", s.trim()]);
        }
    }

    let output = cmd.output().map_err(|e| format!("Failed to execute scrcpy --list-encoders: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}\n{}", stdout, stderr);

    let mut encoders = Vec::new();
    for line in combined.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("--") && (trimmed.contains("audio") || trimmed.contains("opus") || trimmed.contains("aac") || trimmed.contains("flac")) {
            encoders.push(trimmed.to_string());
        }
    }

    if encoders.is_empty() && !output.status.success() {
        let errors: Vec<&str> = combined
            .lines()
            .map(str::trim)
            .filter(|t| t.starts_with("ERROR:"))
            .collect();
        return Err(if errors.is_empty() {
            format!("scrcpy --list-encoders failed ({})", output.status)
        } else {
            errors.join(" ")
        });
    }

    if encoders.is_empty() {
        for line in combined.lines() {
            let t = line.trim();
            if !t.is_empty() && !t.starts_with("scrcpy ") && !t.starts_with("INFO:") && !t.starts_with("WARN:") {
                encoders.push(t.to_string());
            }
        }
    }

    Ok(encoders)
}

pub fn extract_ip_from_inet(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(pos) = trimmed.find("inet ") {
            let mut rest = trimmed[pos + 5..].trim_start();
            if rest.starts_with("addr:") {
                rest = rest[5..].trim_start();
            }
            let ip_candidate = rest.split(['/', ' ']).next().unwrap_or("").trim();
            if let Ok(ip) = ip_candidate.parse::<std::net::Ipv4Addr>() {
                if !ip.is_loopback() && !ip.is_unspecified() {
                    return Some(ip.to_string());
                }
            }
        }
    }
    None
}

pub fn extract_ip_from_route(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.contains("wlan0") && trimmed.contains("src") {
            let tokens: Vec<&str> = trimmed.split_whitespace().collect();
            for i in 0..tokens.len() {
                if tokens[i] == "src" && i + 1 < tokens.len() {
                    if let Ok(ip) = tokens[i + 1].parse::<std::net::Ipv4Addr>() {
                        if !ip.is_loopback() && !ip.is_unspecified() {
                            return Some(ip.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

pub fn extract_ip_from_plain(text: &str) -> Option<String> {
    let candidate = text.trim();
    if let Ok(ip) = candidate.parse::<std::net::Ipv4Addr>() {
        if !ip.is_loopback() && !ip.is_unspecified() {
            return Some(ip.to_string());
        }
    }
    None
}

pub fn get_device_ip(serial: &str, adb_path: Option<&str>) -> Result<String, String> {
    let adb_bin = adb_path.unwrap_or("adb");

    // 1. Primary (escrcpy standard): `ip -f inet addr show wlan0`
    let mut cmd1 = Command::new(adb_bin);
    crate::configure_command(&mut cmd1);
    if let Ok(output) = cmd1.args(["-s", serial, "shell", "ip", "-f", "inet", "addr", "show", "wlan0"]).output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(ip) = extract_ip_from_inet(&stdout) {
                return Ok(ip);
            }
        }
    }

    // 2. Fallback: `ip route`
    let mut cmd2 = Command::new(adb_bin);
    crate::configure_command(&mut cmd2);
    if let Ok(output) = cmd2.args(["-s", serial, "shell", "ip", "route"]).output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(ip) = extract_ip_from_route(&stdout) {
                return Ok(ip);
            }
        }
    }

    // 3. Fallback: `getprop dhcp.wlan0.ipaddress`
    let mut cmd3 = Command::new(adb_bin);
    crate::configure_command(&mut cmd3);
    if let Ok(output) = cmd3.args(["-s", serial, "shell", "getprop", "dhcp.wlan0.ipaddress"]).output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(ip) = extract_ip_from_plain(&stdout) {
                return Ok(ip);
            }
        }
    }

    // 4. Fallback: `ifconfig wlan0`
    let mut cmd4 = Command::new(adb_bin);
    crate::configure_command(&mut cmd4);
    if let Ok(output) = cmd4.args(["-s", serial, "shell", "ifconfig", "wlan0"]).output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(ip) = extract_ip_from_inet(&stdout) {
                return Ok(ip);
            }
        }
    }

    Err(format!(
        "Could not detect Wi-Fi IP address for device '{}'. Ensure the device is connected to Wi-Fi.",
        serial
    ))
}

pub fn enable_tcpip(serial: &str, port: u16, adb_path: Option<&str>) -> Result<String, String> {
    let adb_bin = adb_path.unwrap_or("adb");
    let mut cmd = Command::new(adb_bin);
    crate::configure_command(&mut cmd);
    let output = cmd.args(["-s", serial, "tcpip", &port.to_string()]).output()
        .map_err(|e| format!("Failed to execute adb tcpip: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr).trim().to_string();

    if output.status.success() {
        Ok(if combined.is_empty() { format!("restarting in TCP mode port: {}", port) } else { combined })
    } else {
        Err(format!("adb tcpip failed: {}", combined))
    }
}

pub fn switch_to_wireless(serial: &str, port: Option<u16>, adb_path: Option<&str>) -> Result<String, String> {
    let target_port = port.unwrap_or(5555);
    let ip = get_device_ip(serial, adb_path)?;
    enable_tcpip(serial, target_port, adb_path)?;

    // Allow ADB daemon on Android to initialize TCP/IP socket
    std::thread::sleep(std::time::Duration::from_millis(1000));

    let ip_port = format!("{}:{}", ip, target_port);
    let connect_res = connect_wireless(&ip_port, adb_path)?;

    let _ = save_last_wireless_device(&ip_port);

    Ok(format!("{}: {}", ip_port, connect_res))
}

pub fn disconnect_wireless(target: &str, adb_path: Option<&str>) -> Result<String, String> {
    let adb_bin = adb_path.unwrap_or("adb");
    let mut cmd = Command::new(adb_bin);
    crate::configure_command(&mut cmd);
    let output = cmd.args(["disconnect", target]).output()
        .map_err(|e| format!("Failed to execute adb disconnect: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr).trim().to_string();

    if output.status.success() {
        Ok(if combined.is_empty() { format!("disconnected {}", target) } else { combined })
    } else {
        Err(format!("adb disconnect failed: {}", combined))
    }
}

pub fn pair_device(ip_port: &str, code: &str, adb_path: Option<&str>) -> Result<String, String> {
    let adb_bin = adb_path.unwrap_or("adb");
    let mut cmd = Command::new(adb_bin);
    crate::configure_command(&mut cmd);
    let output = cmd.args(["pair", ip_port, code]).output()
        .map_err(|e| format!("Failed to execute adb pair: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);

    if output.status.success() && !combined.to_lowercase().contains("failed") {
        Ok(combined.trim().to_string())
    } else {
        Err(combined.trim().to_string())
    }
}

pub fn connect_wireless(ip_port: &str, adb_path: Option<&str>) -> Result<String, String> {
    let adb_bin = adb_path.unwrap_or("adb");

    let output = {
        let mut cmd = Command::new(adb_bin);
        crate::configure_command(&mut cmd);
        cmd.arg("connect").arg(ip_port).output()
    }.map_err(|e| format!("Failed to execute adb connect: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr).to_lowercase();
    let failed = combined.contains("failed to connect")
        || combined.contains("unable to connect")
        || combined.contains("cannot connect")
        || combined.contains("connection refused");
    let result = stdout.trim().to_string();
    if output.status.success() && !failed {
        Ok(result)
    } else {
        Err(if result.is_empty() { stderr.trim().to_string() } else { result })
    }
}

pub fn get_last_wireless_device() -> Option<String> {
    let config_path = get_target_dir()?.join("last_wireless_device.txt");
    let device = std::fs::read_to_string(config_path).ok()?.trim().to_string();
    if device.is_empty() { None } else { Some(device) }
}

pub fn save_last_wireless_device(device: &str) -> Result<(), String> {
    let config_path = config_dir()?.join("last_wireless_device.txt");
    std::fs::write(config_path, device.trim()).map_err(|e| format!("Failed to save device: {}", e))
}

/// Directory holding micpy's adb-related settings files, created if missing
/// (it does not exist yet when adb comes from PATH and was never downloaded).
fn config_dir() -> Result<PathBuf, String> {
    let dir = get_target_dir().ok_or("No target directory")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create {:?}: {}", dir, e))?;
    Ok(dir)
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct BatteryInfo {
    pub level: u32,
    pub is_charging: bool,
    pub power_source: String,
    pub temperature: Option<f32>,
    pub health: String,
}

pub fn parse_battery_dump(output: &str) -> Option<BatteryInfo> {
    let mut level: Option<u32> = None;
    let mut scale: u32 = 100;
    let mut ac_powered = false;
    let mut usb_powered = false;
    let mut wireless_powered = false;
    let mut status = 1;
    let mut temp_raw: Option<f32> = None;
    let mut health_code = 2;

    for line in output.lines() {
        let trimmed = line.trim();
        if let Some((k, v)) = trimmed.split_once(':') {
            let key = k.trim().to_lowercase();
            let val = v.trim();
            match key.as_str() {
                "level" => {
                    if let Ok(num) = val.parse::<u32>() {
                        level = Some(num);
                    }
                }
                "scale" => {
                    if let Ok(num) = val.parse::<u32>() {
                        scale = num;
                    }
                }
                "ac powered" => {
                    ac_powered = val.eq_ignore_ascii_case("true");
                }
                "usb powered" => {
                    usb_powered = val.eq_ignore_ascii_case("true");
                }
                "wireless powered" => {
                    wireless_powered = val.eq_ignore_ascii_case("true");
                }
                "status" => {
                    if let Ok(num) = val.parse::<u32>() {
                        status = num;
                    }
                }
                "temperature" => {
                    if let Ok(num) = val.parse::<f32>() {
                        temp_raw = Some(num / 10.0);
                    }
                }
                "health" => {
                    if let Ok(num) = val.parse::<u32>() {
                        health_code = num;
                    }
                }
                _ => {}
            }
        }
    }

    let actual_level = level?;
    let percent = actual_level.saturating_mul(100).checked_div(scale).unwrap_or(actual_level);
    let is_charging = status == 2 || ac_powered || usb_powered || wireless_powered;
    let power_source = if ac_powered {
        "AC".to_string()
    } else if usb_powered {
        "USB".to_string()
    } else if wireless_powered {
        "Wireless".to_string()
    } else {
        "Battery".to_string()
    };

    // android.os.BatteryManager.BATTERY_HEALTH_* constants.
    let health = match health_code {
        2 => "Good",
        3 => "Overheat",
        4 => "Dead",
        5 => "Over Voltage",
        6 => "Failure",
        7 => "Cold",
        _ => "Unknown",
    }.to_string();

    Some(BatteryInfo {
        level: percent,
        is_charging,
        power_source,
        temperature: temp_raw,
        health,
    })
}

pub fn get_battery_info(serial: &str, adb_path: Option<&str>) -> Result<BatteryInfo, String> {
    let adb_bin = adb_path.unwrap_or("adb");
    let mut cmd = Command::new(adb_bin);
    crate::configure_command(&mut cmd);
    let output = cmd.args(["-s", serial, "shell", "dumpsys", "battery"]).output()
        .map_err(|e| format!("Failed to query battery: {}", e))?;

    if !output.status.success() {
        return Err(format!("dumpsys battery failed: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_battery_dump(&stdout).ok_or_else(|| "Could not parse battery status".to_string())
}

pub fn send_keyevent(serial: &str, action: &str, adb_path: Option<&str>) -> Result<String, String> {
    let keycode = match action {
        "wake" | "screen_on" => "224",
        "sleep" | "screen_off" => "223",
        "vol_up" => "24",
        "vol_down" => "25",
        "mute" => "164",
        // `adb shell` joins its arguments into a device-side shell command line,
        // so only pass through plain numeric codes or KEYCODE_* names.
        custom if is_valid_keycode(custom) => custom,
        other => return Err(format!("Invalid keyevent '{}'", other)),
    };

    let adb_bin = adb_path.unwrap_or("adb");
    let mut cmd = Command::new(adb_bin);
    crate::configure_command(&mut cmd);
    let output = cmd.args(["-s", serial, "shell", "input", "keyevent", keycode]).output()
        .map_err(|e| format!("Failed to send keyevent: {}", e))?;

    if output.status.success() {
        Ok(format!("Sent keyevent {} to {}", keycode, serial))
    } else {
        Err(format!("Keyevent failed: {}", String::from_utf8_lossy(&output.stderr).trim()))
    }
}

fn is_valid_keycode(code: &str) -> bool {
    let is_number = !code.is_empty() && code.len() <= 4 && code.bytes().all(|b| b.is_ascii_digit());
    let is_name = code.strip_prefix("KEYCODE_").is_some_and(|rest| {
        !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    });
    is_number || is_name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_ip_from_inet() {
        let sample_wlan0 = r#"
30: wlan0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc mq state UP group default qlen 3000
    inet 192.168.1.135/24 brd 192.168.1.255 scope global wlan0
       valid_lft forever preferred_lft forever
    inet6 fe80::d80a:11ff:fe22:3344/64 scope link
       valid_lft forever preferred_lft forever
"#;
        assert_eq!(extract_ip_from_inet(sample_wlan0), Some("192.168.1.135".to_string()));

        let sample_ifconfig = r#"
wlan0     Link encap:UNSPEC
          inet addr:10.0.0.42  Bcast:10.0.0.255  Mask:255.255.255.0
          UP BROADCAST RUNNING MULTICAST  MTU:1500  Metric:1
"#;
        assert_eq!(extract_ip_from_inet(sample_ifconfig), Some("10.0.0.42".to_string()));
    }

    #[test]
    fn test_extract_ip_from_route() {
        let sample_route = r#"
192.168.0.0/24 dev wlan0 proto kernel scope link src 192.168.0.88
default via 192.168.0.1 dev wlan0
"#;
        assert_eq!(extract_ip_from_route(sample_route), Some("192.168.0.88".to_string()));
    }

    #[test]
    fn test_extract_ip_from_plain() {
        assert_eq!(extract_ip_from_plain("192.168.1.55\n"), Some("192.168.1.55".to_string()));
        assert_eq!(extract_ip_from_plain("127.0.0.1"), None);
        assert_eq!(extract_ip_from_plain("0.0.0.0"), None);
        assert_eq!(extract_ip_from_plain("not_an_ip"), None);
    }

    #[test]
    fn test_parse_battery_dump() {
        let sample = r#"
Current Battery Service state:
  AC powered: false
  USB powered: true
  Wireless powered: false
  status: 2
  health: 2
  present: true
  level: 87
  scale: 100
  voltage: 4150
  temperature: 295
  technology: Li-poly
"#;
        let battery = parse_battery_dump(sample).expect("Should parse battery");
        assert_eq!(battery.level, 87);
        assert!(battery.is_charging);
        assert_eq!(battery.power_source, "USB");
        assert_eq!(battery.temperature, Some(29.5));
        assert_eq!(battery.health, "Good");
    }

    #[test]
    fn test_parse_battery_dump_edge_cases() {
        assert_eq!(parse_battery_dump("scale: 100\nhealth: 2"), None, "missing level");

        let zero_scale = parse_battery_dump("level: 42\nscale: 0\nhealth: 1").unwrap();
        assert_eq!(zero_scale.level, 42);
        assert_eq!(zero_scale.health, "Unknown");

        let half_scale = parse_battery_dump("level: 25\nscale: 50\nhealth: 6").unwrap();
        assert_eq!(half_scale.level, 50);
        assert_eq!(half_scale.health, "Failure");
    }

    #[test]
    fn test_is_valid_keycode() {
        for ok in ["24", "224", "KEYCODE_HOME", "KEYCODE_MEDIA_PLAY_PAUSE", "KEYCODE_F1"] {
            assert!(is_valid_keycode(ok), "{ok} should be accepted");
        }
        for bad in ["", "KEYCODE_", "keycode_home", "12345", "24; reboot", "3 && rm -rf /sdcard", "$(id)", "KEYCODE_HOME;ls"] {
            assert!(!is_valid_keycode(bad), "{bad:?} should be rejected");
        }
        assert!(send_keyevent("serial", "1; reboot", Some("adb")).is_err());
    }
}