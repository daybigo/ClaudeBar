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

/// Ventana semanal acotada a un modelo (del array `limits[]` del endpoint).
/// Aqui llegan Fable, Opus, Sonnet, etc. cuando Anthropic los expone.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopedWindow {
    /// Nombre para mostrar, ej "Fable", "Opus".
    pub model: String,
    /// Id del modelo, ej "claude-fable-5".
    pub model_id: String,
    /// Porcentaje usado 0-100.
    pub utilization: f64,
    pub resets_at: Option<String>,
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
    /// Ventanas semanales por modelo del array `limits[]` (Fable, Opus, ...).
    pub scoped_weekly: Vec<ScopedWindow>,
    pub extra_usage: ExtraUsage,
    /// true si mostramos datos viejos por un error/rate-limit.
    pub stale: bool,
    pub error: Option<String>,
    /// Hora local del ultimo refresco exitoso, ISO 8601.
    pub updated_at: String,
}

/// Costo y tokens de un modelo concreto en la ventana de 30 dias.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    /// Id crudo del modelo, ej "claude-fable-5".
    pub model: String,
    pub cost_usd: f64,
    /// Tokens visibles (sin lectura de cache), como en el resto de la UI.
    pub tokens: u64,
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
    /// Costo USD por dia, ultimos 30 dias (indice 0 = hace 29 dias, 29 = hoy).
    pub daily: Vec<f64>,
    /// Modelo con mas costo en la ventana (ej. "claude-opus-4-8").
    pub top_model: String,
    /// Desglose por modelo (30 dias), ordenado de mayor a menor costo.
    pub models: Vec<ModelUsage>,
    /// Hora local del ultimo calculo, ISO 8601.
    pub updated_at: String,
    /// true si no se encontraron logs (ej. primer arranque).
    pub empty: bool,
}
