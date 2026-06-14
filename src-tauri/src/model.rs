//! Tipos compartidos que se serializan hacia el frontend (camelCase).

use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitWindow {
    /// Porcentaje usado 0-100.
    pub utilization: f64,
    /// ISO 8601 cuando se reinicia la ventana (UTC).
    pub resets_at: Option<String>,
    /// Etiqueta lista para mostrar, ej "3h 53m" o "3d 20h".
    pub resets_in_label: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraUsage {
    pub used_usd: f64,
    pub limit_usd: f64,
    pub utilization: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    /// true si pudimos leer el token de Claude Code.
    pub connected: bool,
    /// "Max", "Pro", etc.
    pub plan: String,
    pub five_hour: LimitWindow,
    pub seven_day: LimitWindow,
    pub seven_day_sonnet: Option<LimitWindow>,
    pub seven_day_opus: Option<LimitWindow>,
    pub extra_usage: ExtraUsage,
    /// true si mostramos datos viejos por un error/rate-limit.
    pub stale: bool,
    pub error: Option<String>,
    /// Hora local del ultimo refresco exitoso, ISO 8601.
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CostReport {
    pub today_usd: f64,
    pub today_tokens: u64,
    pub week_usd: f64,
    pub week_tokens: u64,
    pub month_usd: f64,
    pub month_tokens: u64,
    pub last30_usd: f64,
    pub last30_tokens: u64,
    /// Hora local del ultimo calculo, ISO 8601.
    pub updated_at: String,
    /// true si no se encontraron logs (ej. primer arranque).
    pub empty: bool,
}
