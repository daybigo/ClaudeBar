//! Lectura de las credenciales de Claude Code en Windows.
//! Archivo: %USERPROFILE%\.claude\.credentials.json
//!
//! El token de acceso caduca cada ~60 min, pero Claude Code lo refresca solo
//! mientras se usa. Por eso lo releemos del archivo en cada llamada.

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct CredentialsFile {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<OauthBlock>,
}

#[derive(Debug, Deserialize)]
struct OauthBlock {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
    #[serde(rename = "expiresAt")]
    expires_at: Option<i64>,
    #[serde(rename = "subscriptionType")]
    subscription_type: Option<String>,
    #[serde(rename = "rateLimitTier")]
    rate_limit_tier: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Credentials {
    pub access_token: String,
    pub plan: String,
    /// epoch en milisegundos (puede ser 0 si no esta presente).
    pub expires_at: i64,
}

/// Ruta del directorio de configuracion de Claude (respeta CLAUDE_CONFIG_DIR).
pub fn claude_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !custom.trim().is_empty() {
            return PathBuf::from(custom);
        }
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".claude")
}

/// Convierte "max" -> "Max", "pro" -> "Pro", etc.
fn pretty_plan(raw: &str) -> String {
    match raw.to_ascii_lowercase().as_str() {
        "max" => "Max".to_string(),
        "pro" => "Pro".to_string(),
        "free" => "Free".to_string(),
        other if !other.is_empty() => {
            let mut c = other.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => other.to_string(),
            }
        }
        _ => "Claude".to_string(),
    }
}

/// Extrae el multiplicador del tier, ej "default_claude_max_20x" -> "20x".
fn tier_multiplier(tier: &str) -> Option<String> {
    tier.split('_').find_map(|tok| {
        let t = tok.to_ascii_lowercase();
        if t.len() >= 2 && t.ends_with('x') {
            let num = &t[..t.len() - 1];
            if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
                return Some(t);
            }
        }
        None
    })
}

/// Combina suscripcion + tier en una etiqueta exacta, ej "Max 20x".
fn format_plan(subscription: &str, tier: &str) -> String {
    let base = pretty_plan(subscription);
    match tier_multiplier(tier) {
        Some(m) => format!("{base} {m}"),
        None => base,
    }
}

/// Lee las credenciales. Devuelve None si no hay token (Claude Code no
/// conectado).
pub fn read() -> Option<Credentials> {
    let path = claude_dir().join(".credentials.json");
    let bytes = std::fs::read(&path).ok()?;
    let parsed: CredentialsFile = serde_json::from_slice(&bytes).ok()?;
    let oauth = parsed.claude_ai_oauth?;
    let token = oauth.access_token?;
    if token.trim().is_empty() {
        return None;
    }
    let plan = format_plan(
        oauth.subscription_type.as_deref().unwrap_or(""),
        oauth.rate_limit_tier.as_deref().unwrap_or(""),
    );
    Some(Credentials {
        access_token: token,
        plan,
        expires_at: oauth.expires_at.unwrap_or(0),
    })
}
