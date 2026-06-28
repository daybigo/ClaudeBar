//! Lectura del estado de Antigravity (Google) desde su `state.vscdb`.
//!
//! La clave `antigravityAuthStatus` guarda un JSON con name/email y un blob
//! protobuf en base64 (`userStatusProtoBinaryBase64`) que contiene el plan.
//! No tenemos el esquema proto, asi que extraemos el nombre del plan buscando
//! la cadena "Google AI ..." dentro de los bytes decodificados.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;

/// Una cubeta de cuota de Antigravity (p.ej. "Gemini Models" / "Weekly Limit").
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityBucket {
    pub group: String,       // "Gemini Models" / "Claude and GPT models"
    pub label: String,       // "Weekly Limit" / "Five Hour Limit"
    pub window: String,      // "weekly" | "5h"
    pub used_percent: f64,
    pub resets_at: i64,      // epoch en segundos (0 si no se conoce)
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct AntigravityStatus {
    pub connected: bool,
    pub email: String,
    pub plan: String,
    pub buckets: Vec<AntigravityBucket>,
}

fn db_path() -> Option<PathBuf> {
    // Carpeta base de config de Antigravity (un fork de VS Code):
    //   Windows: %APPDATA%\Antigravity
    //   Linux:   ~/.config/Antigravity
    let base = if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else {
        dirs::config_dir()?
    };
    let p = base
        .join("Antigravity")
        .join("User")
        .join("globalStorage")
        .join("state.vscdb");
    p.exists().then_some(p)
}

pub fn read() -> AntigravityStatus {
    let Some(path) = db_path() else {
        return AntigravityStatus::default();
    };
    let Some(raw) = crate::vscdb::read_item(&path, "antigravityAuthStatus") else {
        return AntigravityStatus::default();
    };
    let json: serde_json::Value = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(_) => return AntigravityStatus::default(),
    };
    let email = json
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let plan = json
        .get("userStatusProtoBinaryBase64")
        .and_then(|v| v.as_str())
        .and_then(base64_decode)
        .and_then(|bytes| extract_plan(&bytes))
        .unwrap_or_else(|| "Antigravity".to_string());

    AntigravityStatus {
        connected: true,
        email,
        plan,
        buckets: read_quota().unwrap_or_default(),
    }
}

/// Lee el uso real desde el `language_server` local de la app de Antigravity
/// (mismo origen que usa la propia IDE; solo funciona si la app esta abierta).
fn read_quota() -> Option<Vec<AntigravityBucket>> {
    let (csrf, ports) = find_language_server()?;
    let client = reqwest::blocking::Client::builder()
        .danger_accept_invalid_certs(true) // cert self-signed en 127.0.0.1
        .timeout(Duration::from_secs(5))
        .build()
        .ok()?;
    for port in ports {
        let url = format!(
            "https://127.0.0.1:{port}/exa.language_server_pb.LanguageServerService/RetrieveUserQuotaSummary"
        );
        let resp = client
            .post(&url)
            .header("X-Codeium-Csrf-Token", &csrf)
            .header("Connect-Protocol-Version", "1")
            .header("Content-Type", "application/json")
            .body("{}")
            .send();
        if let Ok(r) = resp {
            if r.status().is_success() {
                if let Ok(v) = r.json::<serde_json::Value>() {
                    let buckets = parse_quota(&v);
                    if !buckets.is_empty() {
                        return Some(buckets);
                    }
                }
            }
        }
    }
    None
}

fn parse_quota(v: &serde_json::Value) -> Vec<AntigravityBucket> {
    let mut out = Vec::new();
    let Some(groups) = v
        .get("response")
        .and_then(|r| r.get("groups"))
        .and_then(|g| g.as_array())
    else {
        return out;
    };
    for g in groups {
        let group = g
            .get("displayName")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let Some(buckets) = g.get("buckets").and_then(|b| b.as_array()) else {
            continue;
        };
        for b in buckets {
            let remaining = b
                .get("remainingFraction")
                .and_then(|x| x.as_f64())
                .unwrap_or(1.0);
            let resets_at = b
                .get("resetTime")
                .and_then(|x| x.as_str())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.timestamp())
                .unwrap_or(0);
            out.push(AntigravityBucket {
                group: group.clone(),
                label: b.get("displayName").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                window: b.get("window").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                used_percent: ((1.0 - remaining) * 100.0).clamp(0.0, 100.0),
                resets_at,
            });
        }
    }
    out
}

/// Localiza el proceso `language_server` de Antigravity y devuelve (csrf, puertos).
/// Solo Windows por ahora; en otros SO devuelve None (sin barras de uso).
#[cfg(windows)]
fn find_language_server() -> Option<(String, Vec<u16>)> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let script = r#"$p = Get-CimInstance Win32_Process | Where-Object { $_.Name -eq 'language_server.exe' -and $_.CommandLine -like '*antigravity*' } | Select-Object -First 1
if ($p) {
  $csrf = ([regex]'--csrf_token[ =]+([^ ]+)').Match($p.CommandLine).Groups[1].Value
  $ports = (Get-NetTCPConnection -State Listen -OwningProcess $p.ProcessId -ErrorAction SilentlyContinue | Select-Object -ExpandProperty LocalPort -Unique) -join ','
  Write-Output "$csrf|$ports"
}"#;
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let (csrf, ports_str) = s.trim().split_once('|')?;
    if csrf.is_empty() {
        return None;
    }
    let ports: Vec<u16> = ports_str
        .split(',')
        .filter_map(|p| p.trim().parse::<u16>().ok())
        .collect();
    if ports.is_empty() {
        return None;
    }
    Some((csrf.to_string(), ports))
}

#[cfg(not(windows))]
fn find_language_server() -> Option<(String, Vec<u16>)> {
    None
}

/// Decodificador base64 estandar (suficiente para este blob; ignora espacios).
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut buf = 0u32;
    let mut bits = 0;
    for &c in s.as_bytes() {
        if c == b'=' || c.is_ascii_whitespace() {
            continue;
        }
        let v = val(c)?;
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

/// Busca el nombre del plan ("Google AI Pro", "Google AI Ultra", ...) en el blob.
fn extract_plan(bytes: &[u8]) -> Option<String> {
    let needle = b"Google AI ";
    let pos = bytes.windows(needle.len()).position(|w| w == needle)?;
    let mut end = pos + needle.len();
    while end < bytes.len() && (bytes[end].is_ascii_alphabetic() || bytes[end] == b' ') {
        end += 1;
    }
    let s = String::from_utf8_lossy(&bytes[pos..end]).trim().to_string();
    (!s.is_empty()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_redondea() {
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode("R29vZ2xlIEFJIFBybw==").unwrap(), b"Google AI Pro");
    }

    #[test]
    fn extrae_el_plan() {
        let blob = b"\x0a\x05junkGoogle AI Pro:%https://upgrade";
        assert_eq!(extract_plan(blob).unwrap(), "Google AI Pro");
    }

    #[test]
    fn sin_plan_devuelve_none() {
        assert!(extract_plan(b"sin plan aqui").is_none());
    }
}
