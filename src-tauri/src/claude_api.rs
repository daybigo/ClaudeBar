//! Llama al endpoint de uso de la suscripcion de Claude.
//!
//! GET https://api.anthropic.com/api/oauth/usage
//! Headers obligatorios:
//!   Authorization: Bearer <accessToken>
//!   anthropic-beta: oauth-2025-04-20
//!   User-Agent: claude-code/<version>   (sin esto: 429 persistentes)
//!
//! El endpoint limita agresivamente; el llamador debe espaciar los polls.

use crate::credentials::{claude_dir, Credentials};
use crate::model::{ExtraUsage, LimitWindow, UsageSnapshot};
use chrono::Utc;
use serde_json::Value;
use std::sync::OnceLock;
use std::time::Duration;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const ANTHROPIC_BETA: &str = "oauth-2025-04-20";
const FALLBACK_VERSION: &str = "2.1.139";

/// Resultado de un intento de refresco.
pub enum FetchResult {
    Ok(UsageSnapshot),
    /// 429: hay que aplicar backoff.
    RateLimited,
    /// Otro error (red, token caducado, parseo).
    Error(String),
}

static UA: OnceLock<String> = OnceLock::new();

/// User-Agent estilo Claude Code, con la version detectada de los logs.
fn user_agent() -> &'static str {
    UA.get_or_init(|| format!("claude-code/{}", detect_cli_version()))
}

/// Busca el campo "version" en el .jsonl mas reciente bajo projects/.
fn detect_cli_version() -> String {
    let projects = claude_dir().join("projects");
    let mut newest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for entry in walkdir::WalkDir::new(&projects)
        .max_depth(4)
        .into_iter()
        .flatten()
    {
        if entry.file_type().is_file()
            && entry.path().extension().map(|e| e == "jsonl").unwrap_or(false)
        {
            if let Ok(meta) = entry.metadata() {
                if let Ok(modified) = meta.modified() {
                    if newest.as_ref().map(|(t, _)| modified > *t).unwrap_or(true) {
                        newest = Some((modified, entry.path().to_path_buf()));
                    }
                }
            }
        }
    }
    if let Some((_, path)) = newest {
        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines().rev().take(50) {
                if let Ok(v) = serde_json::from_str::<Value>(line) {
                    if let Some(ver) = v.get("version").and_then(|x| x.as_str()) {
                        if !ver.is_empty() {
                            return ver.to_string();
                        }
                    }
                }
            }
        }
    }
    FALLBACK_VERSION.to_string()
}

/// Convierte un ISO 8601 futuro en una etiqueta tipo "3h 53m" / "3d 20h".
pub fn resets_in_label(iso: &str) -> String {
    let parsed = chrono::DateTime::parse_from_rfc3339(iso);
    let target = match parsed {
        Ok(dt) => dt.with_timezone(&Utc),
        Err(_) => return String::new(),
    };
    let now = Utc::now();
    let secs = (target - now).num_seconds();
    if secs <= 0 {
        return "ahora".to_string();
    }
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

/// Extrae un LimitWindow de un objeto {utilization, resets_at}.
fn parse_window(v: &Value) -> Option<LimitWindow> {
    let obj = v.as_object()?;
    let utilization = obj
        .get("utilization")
        .and_then(|x| x.as_f64())
        .unwrap_or(0.0);
    let resets_at = obj
        .get("resets_at")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    let resets_in_label = resets_at.as_deref().map(resets_in_label).unwrap_or_default();
    Some(LimitWindow {
        utilization,
        resets_at,
        resets_in_label,
    })
}

/// Lee el bloque extra_usage de forma tolerante (la forma exacta no esta
/// documentada). Busca varios nombres de campo plausibles.
fn parse_extra_usage(v: &Value) -> ExtraUsage {
    let pick = |obj: &Value, keys: &[&str]| -> Option<f64> {
        for k in keys {
            if let Some(n) = obj.get(*k).and_then(|x| x.as_f64()) {
                return Some(n);
            }
        }
        None
    };
    // Nombres reales del endpoint primero, luego alternativas tolerantes.
    let used = pick(
        v,
        &["used_credits", "used_usd", "used", "spent", "amount", "current"],
    )
    .unwrap_or(0.0);
    let limit = pick(
        v,
        &["monthly_limit", "limit_usd", "limit", "cap", "max", "budget"],
    )
    .unwrap_or(0.0);
    let utilization = pick(v, &["utilization"]).unwrap_or_else(|| {
        if limit > 0.0 {
            (used / limit) * 100.0
        } else {
            0.0
        }
    });
    ExtraUsage {
        used_usd: used,
        limit_usd: limit,
        utilization,
    }
}

/// Hace la peticion HTTP y normaliza la respuesta.
pub fn fetch(creds: &Credentials) -> FetchResult {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
    {
        Ok(c) => c,
        Err(e) => return FetchResult::Error(format!("cliente HTTP: {e}")),
    };

    let resp = client
        .get(USAGE_URL)
        .header("Authorization", format!("Bearer {}", creds.access_token))
        .header("anthropic-beta", ANTHROPIC_BETA)
        .header("User-Agent", user_agent())
        .header("Content-Type", "application/json")
        .send();

    let resp = match resp {
        Ok(r) => r,
        Err(_) => return FetchResult::Error("network".to_string()),
    };

    // Devolvemos CODIGOS de error (el frontend los traduce al idioma del usuario).
    let status = resp.status();
    if status.as_u16() == 429 {
        return FetchResult::RateLimited;
    }
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return FetchResult::Error("session_expired".to_string());
    }
    if !status.is_success() {
        return FetchResult::Error(format!("http_{}", status.as_u16()));
    }

    let body: Value = match resp.json() {
        Ok(v) => v,
        Err(_) => return FetchResult::Error("parse_error".to_string()),
    };

    let five_hour = body.get("five_hour").and_then(parse_window).unwrap_or_default();
    let seven_day = body.get("seven_day").and_then(parse_window).unwrap_or_default();
    let seven_day_sonnet = body.get("seven_day_sonnet").and_then(parse_window);
    let seven_day_opus = body.get("seven_day_opus").and_then(parse_window);
    let extra_usage = body
        .get("extra_usage")
        .map(parse_extra_usage)
        .unwrap_or_default();

    FetchResult::Ok(UsageSnapshot {
        connected: true,
        plan: creds.plan.clone(),
        five_hour,
        seven_day,
        seven_day_sonnet,
        seven_day_opus,
        extra_usage,
        stale: false,
        error: None,
        updated_at: chrono::Local::now().to_rfc3339(),
    })
}
