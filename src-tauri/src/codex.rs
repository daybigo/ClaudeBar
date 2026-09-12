//! Lectura del estado de Codex (ChatGPT) desde `~/.codex/auth.json`.
//!
//! El plan vive en los claims del `id_token` (un JWT): el campo
//! `https://api.openai.com/auth.chatgpt_plan_type` ("plus", "pro", ...).
//! Decodificamos el payload del JWT (base64url) y leemos email + plan.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use walkdir::WalkDir;
use crate::codex_cost::{CostCache, CostReport};

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodexWindow {
    pub used_percent: f64,
    /// Duración de la ventana en minutos (300=5h, 10080=semana, 43200=mes).
    pub window_minutes: i64,
    /// Epoch en segundos en que se reinicia la ventana.
    pub resets_at: i64,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodexStatus {
    pub connected: bool,
    pub email: String,
    pub plan: String,
    pub primary: Option<CodexWindow>,
    pub secondary: Option<CodexWindow>,
    pub additional: Vec<NamedLimit>,
    pub credits: Option<Credits>,
    pub cost: Option<CostReport>,
    pub usage_source: String,
    pub usage_updated_at: String,
    pub usage_error: Option<String>,
}

#[derive(Serialize, Clone, Default)]
pub struct NamedLimit {
    pub label: String,
    pub primary: Option<CodexWindow>,
    pub secondary: Option<CodexWindow>,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Credits {
    pub balance: Option<f64>,
    pub unlimited: bool,
    pub has_credits: bool,
}

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME").filter(|s| !s.is_empty()).map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|p| p.join(".codex")))
}

#[derive(Default)]
struct Reader {
    status: CodexStatus,
    costs: CostCache,
    account: String,
    last_scan: Option<Instant>,
    last_fetch: Option<Instant>,
}

pub fn read(force: bool) -> CodexStatus {
    static READER: OnceLock<Mutex<Reader>> = OnceLock::new();
    let mut reader = READER.get_or_init(|| Mutex::new(Reader::default())).lock().unwrap();
    let Some(home) = codex_home() else {
        return CodexStatus::default();
    };
    let Ok(bytes) = std::fs::read(home.join("auth.json")) else {
        *reader = Reader::default();
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

    let access_token = root.pointer("/tokens/access_token").and_then(|v| v.as_str()).unwrap_or("");
    let account_id = root.pointer("/tokens/account_id").and_then(|v| v.as_str()).unwrap_or("");
    let account = format!("{}:{}:{}", home.display(), account_id, email);
    if reader.account != account {
        *reader = Reader { account, ..Default::default() };
    }
    if !force && reader.last_scan.is_some_and(|t| t.elapsed() < Duration::from_secs(60)) {
        return reader.status.clone();
    }
    reader.status.connected = !access_token.is_empty();
    reader.status.email = email;
    if reader.status.plan.is_empty() { reader.status.plan = label_plan(plan_type); }
    if !access_token.is_empty() && reader.last_fetch.is_none_or(|t| {
        t.elapsed() >= Duration::from_secs(if force { 20 } else { 300 })
    }) {
        reader.last_fetch = Some(Instant::now());
        match fetch_usage(access_token, account_id) {
            Ok(value) => {
                apply_api_usage(&mut reader.status, &value);
                reader.status.usage_source = "api".to_string();
                reader.status.usage_updated_at = chrono::Local::now().to_rfc3339();
                reader.status.usage_error = None;
            }
            Err(error) => reader.status.usage_error = Some(error.to_string()),
        }
    }
    if reader.status.usage_source != "api" {
        let (primary, secondary) = read_usage(&home);
        reader.status.primary = primary;
        reader.status.secondary = secondary;
        reader.status.usage_source = "local".to_string();
    }
    reader.status.cost = Some(reader.costs.compute(&home));
    reader.last_scan = Some(Instant::now());
    reader.status.clone()
}

fn fetch_usage(token: &str, account_id: &str) -> Result<serde_json::Value, &'static str> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build().map_err(|_| "network")?;
    let mut request = client.get("https://chatgpt.com/backend-api/wham/usage")
        .bearer_auth(token);
    if !account_id.is_empty() { request = request.header("ChatGPT-Account-Id", account_id); }
    let response = request.send().map_err(|_| "network")?;
    match response.status().as_u16() {
        200 => {
            let value: serde_json::Value = response.json().map_err(|_| "parse_error")?;
            if !value.get("rate_limit").is_some_and(|v| v.is_object()) {
                return Err("parse_error");
            }
            Ok(value)
        }
        401 | 403 => Err("session_expired"),
        429 => Err("rate_limited"),
        _ => Err("network"),
    }
}

fn api_window(v: &serde_json::Value) -> Option<CodexWindow> {
    let used = v.get("used_percent")?.as_f64()?;
    if !used.is_finite() { return None; }
    Some(CodexWindow {
        used_percent: used,
        window_minutes: v["limit_window_seconds"].as_i64().unwrap_or(0) / 60,
        resets_at: v["reset_at"].as_i64().unwrap_or(0),
    })
}

fn apply_api_usage(status: &mut CodexStatus, value: &serde_json::Value) {
    if let Some(plan) = value["plan_type"].as_str() { status.plan = label_plan(plan); }
    let limits = &value["rate_limit"];
    status.primary = api_window(&limits["primary_window"]);
    status.secondary = api_window(&limits["secondary_window"]);
    status.additional.clear();
    if let Some(additional) = value["additional_rate_limits"].as_array() {
        for limit in additional {
            status.additional.push(NamedLimit {
                label: limit["limit_name"].as_str().unwrap_or("Codex").to_string(),
                primary: api_window(&limit["rate_limit"]["primary_window"]),
                secondary: api_window(&limit["rate_limit"]["secondary_window"]),
            });
        }
    }
    let review = &value["code_review_rate_limit"];
    if review.is_object() {
        status.additional.push(NamedLimit {
            label: "Code review".to_string(),
            primary: api_window(&review["primary_window"]),
            secondary: api_window(&review["secondary_window"]),
        });
    }
    status.credits = value.get("credits").filter(|v| v.is_object()).map(|v| Credits {
        balance: v["balance"].as_f64().or_else(|| v["balance"].as_str()?.parse().ok())
            .filter(|n: &f64| n.is_finite()),
        unlimited: v["unlimited"].as_bool().unwrap_or(false),
        has_credits: v["has_credits"].as_bool().unwrap_or(false),
    });
}

/// Lee el uso (rate limits) del rollout de sesion mas reciente. Codex escribe
/// eventos con `rate_limits` (primary/secondary) en ~/.codex/sessions/**.
fn read_usage(home: &Path) -> (Option<CodexWindow>, Option<CodexWindow>) {
    let dir = home.join("sessions");
    if !dir.is_dir() {
        return (None, None);
    }
    // rollouts ordenados por fecha de modificacion (mas reciente primero)
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = WalkDir::new(&dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file() && {
                let n = e.file_name().to_string_lossy();
                n.starts_with("rollout-") && n.ends_with(".jsonl")
            }
        })
        .filter_map(|e| {
            let m = e.metadata().ok()?.modified().ok()?;
            Some((m, e.into_path()))
        })
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));

    for (_, path) in files.iter().take(6) {
        if let Some(r) = last_rate_limits(path) {
            return r;
        }
    }
    (None, None)
}

/// Busca el ultimo evento con `rate_limits` en un rollout (filtra por substring
/// para no parsear cada linea de archivos que pueden pesar decenas de MB).
fn last_rate_limits(path: &Path) -> Option<(Option<CodexWindow>, Option<CodexWindow>)> {
    let bytes = std::fs::read(path).ok()?;
    let content = String::from_utf8_lossy(&bytes);
    let mut found = None;
    for line in content.lines() {
        if !line.contains("rate_limits") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v["type"] != "event_msg" || v["payload"]["type"] != "token_count" { continue; }
        if let Some(rl) = v.pointer("/payload/rate_limits") {
            if rl["limit_id"].as_str().is_some_and(|id| id != "codex") { continue; }
            let p = rl.get("primary").and_then(parse_window);
            let s = rl.get("secondary").and_then(parse_window);
            if p.is_some() || s.is_some() {
                found = Some((p, s));
            }
        }
    }
    found
}

fn parse_window(v: &serde_json::Value) -> Option<CodexWindow> {
    let o = v.as_object()?;
    let used = o.get("used_percent")?.as_f64()?;
    Some(CodexWindow {
        used_percent: used,
        window_minutes: o.get("window_minutes").and_then(|x| x.as_i64()).unwrap_or(0),
        resets_at: o.get("resets_at").and_then(|x| x.as_i64()).unwrap_or(0),
    })
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

    #[test]
    fn api_usage_preserves_real_windows_and_credit_units() {
        let mut status = CodexStatus::default();
        apply_api_usage(&mut status, &serde_json::json!({
            "plan_type": "prolite",
            "rate_limit": { "primary_window": {
                "used_percent": 35, "limit_window_seconds": 604800, "reset_at": 1789805789
            }, "secondary_window": null },
            "additional_rate_limits": [{ "limit_name": "GPT-5.3-Codex-Spark", "rate_limit": {
                "primary_window": { "used_percent": 0, "limit_window_seconds": 18000, "reset_at": 1789266118 }
            }}],
            "credits": { "balance": "12.5", "has_credits": true, "unlimited": false }
        }));
        assert_eq!(status.primary.as_ref().unwrap().window_minutes, 10080);
        assert!(status.secondary.is_none());
        assert_eq!(status.additional[0].primary.as_ref().unwrap().window_minutes, 300);
        assert_eq!(status.credits.as_ref().unwrap().balance, Some(12.5));
        let serialized = serde_json::to_value(status).unwrap();
        assert!(serialized.get("usageSource").is_some());
        assert!(serialized.get("accessToken").is_none());
    }

    #[test]
    fn api_usage_clears_removed_additional_windows_and_credits() {
        let mut status = CodexStatus { additional: vec![NamedLimit::default()],
            credits: Some(Credits::default()), ..Default::default() };
        apply_api_usage(&mut status, &serde_json::json!({"rate_limit": {}}));
        assert!(status.additional.is_empty());
        assert!(status.credits.is_none());
        assert!(status.primary.is_none());
    }

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
