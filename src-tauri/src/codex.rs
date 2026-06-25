//! Lectura del estado de Codex (ChatGPT) desde `~/.codex/auth.json`.
//!
//! El plan vive en los claims del `id_token` (un JWT): el campo
//! `https://api.openai.com/auth.chatgpt_plan_type` ("plus", "pro", ...).
//! Decodificamos el payload del JWT (base64url) y leemos email + plan.

use std::path::PathBuf;

use serde::Serialize;

#[derive(Serialize, Clone, Debug, Default)]
pub struct CodexStatus {
    pub connected: bool,
    pub email: String,
    pub plan: String,
}

fn auth_path() -> Option<PathBuf> {
    let p = dirs::home_dir()?.join(".codex").join("auth.json");
    p.exists().then_some(p)
}

pub fn read() -> CodexStatus {
    let Some(path) = auth_path() else {
        return CodexStatus::default();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return CodexStatus::default();
    };
    let Ok(root) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return CodexStatus::default();
    };
    let id_token = root
        .get("tokens")
        .and_then(|t| t.get("id_token"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let claims = decode_jwt_claims(id_token).unwrap_or(serde_json::Value::Null);
    let email = claims
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let plan_type = claims
        .get("https://api.openai.com/auth")
        .and_then(|a| a.get("chatgpt_plan_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    CodexStatus {
        connected: true, // hay auth.json => sesion de Codex presente
        email,
        plan: label_plan(plan_type),
    }
}

/// Decodifica el payload (claims) de un JWT sin verificar la firma.
fn decode_jwt_claims(jwt: &str) -> Option<serde_json::Value> {
    let payload = jwt.split('.').nth(1)?;
    let bytes = base64_decode(payload)?;
    serde_json::from_slice(&bytes).ok()
}

/// Nombre legible del plan a partir de `chatgpt_plan_type`.
fn label_plan(t: &str) -> String {
    match t {
        "" => "ChatGPT".to_string(),
        "free" => "ChatGPT Free".to_string(),
        "plus" => "ChatGPT Plus".to_string(),
        "pro" => "ChatGPT Pro".to_string(),
        "team" => "ChatGPT Team".to_string(),
        "enterprise" => "ChatGPT Enterprise".to_string(),
        other => {
            let mut c = other.chars();
            let head = c.next().map(|f| f.to_uppercase().to_string()).unwrap_or_default();
            format!("ChatGPT {}{}", head, c.as_str())
        }
    }
}

/// base64 que acepta el alfabeto estandar (+/) y el url-safe (-_); ignora padding.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' | b'-' => Some(62),
            b'/' | b'_' => Some(63),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_jwt(payload_json: &str) -> String {
        // header.payload.sig (firma irrelevante; no se verifica)
        let b64 = |b: &[u8]| {
            const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
            let mut o = String::new();
            for ch in b.chunks(3) {
                let n = (ch[0] as u32) << 16
                    | (*ch.get(1).unwrap_or(&0) as u32) << 8
                    | (*ch.get(2).unwrap_or(&0) as u32);
                for i in 0..ch.len() + 1 {
                    o.push(A[((n >> (18 - 6 * i)) & 0x3f) as usize] as char);
                }
            }
            o
        };
        format!("{}.{}.{}", b64(b"{}"), b64(payload_json.as_bytes()), "sig")
    }

    #[test]
    fn decodifica_claims_y_plan() {
        let jwt = make_jwt(
            r#"{"email":"a@b.com","https://api.openai.com/auth":{"chatgpt_plan_type":"plus"}}"#,
        );
        let claims = decode_jwt_claims(&jwt).expect("claims");
        assert_eq!(claims.get("email").unwrap().as_str().unwrap(), "a@b.com");
        let pt = claims["https://api.openai.com/auth"]["chatgpt_plan_type"]
            .as_str()
            .unwrap();
        assert_eq!(label_plan(pt), "ChatGPT Plus");
    }

    #[test]
    fn etiqueta_de_plan() {
        assert_eq!(label_plan("pro"), "ChatGPT Pro");
        assert_eq!(label_plan(""), "ChatGPT");
        assert_eq!(label_plan("business"), "ChatGPT Business");
    }

    #[test]
    fn base64_url_y_estandar() {
        assert_eq!(base64_decode("aGVsbG8").unwrap(), b"hello"); // sin padding
        assert_eq!(base64_decode("Pz8_Pw").unwrap(), b"????"); // url-safe (_)
    }
}
