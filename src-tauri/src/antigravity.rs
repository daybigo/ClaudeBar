//! Lectura del estado de Antigravity (Google) desde su `state.vscdb`.
//!
//! La clave `antigravityAuthStatus` guarda un JSON con name/email y un blob
//! protobuf en base64 (`userStatusProtoBinaryBase64`) que contiene el plan.
//! No tenemos el esquema proto, asi que extraemos el nombre del plan buscando
//! la cadena "Google AI ..." dentro de los bytes decodificados.

use std::path::PathBuf;

use serde::Serialize;

#[derive(Serialize, Clone, Debug, Default)]
pub struct AntigravityStatus {
    pub connected: bool,
    pub email: String,
    pub plan: String,
}

fn db_path() -> Option<PathBuf> {
    // Windows: %APPDATA%\Antigravity\User\globalStorage\state.vscdb
    let appdata = std::env::var_os("APPDATA")?;
    let p = PathBuf::from(appdata)
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
    }
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
