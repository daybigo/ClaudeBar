//! Calcula el costo a partir de los logs locales de Claude Code.
//!
//! Parsea %USERPROFILE%\.claude\projects\**\*.jsonl, igual que ccusage:
//!  - solo lineas con type=="assistant" que traen message.usage
//!  - deduplica por requestId (se queda con el registro mas completo)
//!  - multiplica tokens por el precio del modelo
//!
//! Nota: Claude Code no guarda el costo real, asi que esto es una ESTIMACION.
//! Los tokens "mostrados" excluyen la lectura de cache (que es enorme y barata)
//! para que el numero se parezca al de los dashboards oficiales.

use crate::credentials::claude_dir;
use crate::model::CostReport;
use crate::pricing;
use chrono::{DateTime, Datelike, Duration, Local, Utc};
use serde_json::Value;
use std::collections::HashMap;

/// Solo miramos archivos modificados en los ultimos N dias (cubre hoy / semana
/// / mes-calendario de 31d / 30d rolling). Acelera mucho el escaneo. Margen
/// extra por desfase de mtime de OneDrive y zonas horarias.
const WINDOW_DAYS: i64 = 35;

struct Record {
    ts: DateTime<Utc>,
    model: String,
    input: u64,
    output: u64,
    cache_create_5m: u64,
    cache_create_1h: u64,
    cache_read: u64,
}

impl Record {
    fn total_tokens(&self) -> u64 {
        self.input + self.output + self.cache_create_5m + self.cache_create_1h + self.cache_read
    }
    /// Tokens "visibles" (sin lectura de cache).
    fn display_tokens(&self) -> u64 {
        self.input + self.output + self.cache_create_5m + self.cache_create_1h
    }
    fn cost(&self) -> f64 {
        pricing::cost_usd(
            &self.model,
            self.input,
            self.output,
            self.cache_create_5m,
            self.cache_create_1h,
            self.cache_read,
        )
    }
}

fn u64_at(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(|x| x.as_u64()).unwrap_or(0)
}

/// Parsea una linea jsonl a Record (o None si no es un assistant con usage).
fn parse_line(line: &str) -> Option<(String, Record)> {
    if !line.contains("\"assistant\"") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("type").and_then(|x| x.as_str()) != Some("assistant") {
        return None;
    }
    let msg = v.get("message")?;
    let usage = msg.get("usage")?;

    let ts_str = v.get("timestamp").and_then(|x| x.as_str())?;
    let ts = DateTime::parse_from_rfc3339(ts_str).ok()?.with_timezone(&Utc);

    let model = msg
        .get("model")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown")
        .to_string();

    // Desglose de cache_creation si existe; si no, todo va al bucket de 5m.
    let cc_total = u64_at(usage, "cache_creation_input_tokens");
    let (cc_5m, cc_1h) = match usage.get("cache_creation") {
        Some(b) if b.is_object() => (
            u64_at(b, "ephemeral_5m_input_tokens"),
            u64_at(b, "ephemeral_1h_input_tokens"),
        ),
        _ => (cc_total, 0),
    };
    // Si el desglose no cuadra con el total, usamos el total como 5m.
    let (cc_5m, cc_1h) = if cc_5m + cc_1h == 0 && cc_total > 0 {
        (cc_total, 0)
    } else {
        (cc_5m, cc_1h)
    };

    let record = Record {
        ts,
        model,
        input: u64_at(usage, "input_tokens"),
        output: u64_at(usage, "output_tokens"),
        cache_create_5m: cc_5m,
        cache_create_1h: cc_1h,
        cache_read: u64_at(usage, "cache_read_input_tokens"),
    };

    // Clave de deduplicacion: requestId; si no, el uuid de la linea.
    let key = v
        .get("requestId")
        .and_then(|x| x.as_str())
        .or_else(|| v.get("uuid").and_then(|x| x.as_str()))
        .unwrap_or("")
        .to_string();
    if key.is_empty() {
        return None;
    }
    Some((key, record))
}

/// Recorre los logs y agrega el costo en ventanas de tiempo.
pub fn compute() -> CostReport {
    let projects = claude_dir().join("projects");
    let cutoff_file = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs((WINDOW_DAYS as u64) * 86_400))
        .unwrap_or(std::time::UNIX_EPOCH);

    // requestId -> registro mas completo (mayor cantidad de tokens).
    let mut records: HashMap<String, Record> = HashMap::new();

    for entry in walkdir::WalkDir::new(&projects)
        .into_iter()
        .flatten()
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().map(|e| e != "jsonl").unwrap_or(true) {
            continue;
        }
        // Salta archivos viejos (fuera de la ventana de 32 dias).
        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                if modified < cutoff_file {
                    continue;
                }
            }
        }
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for line in content.lines() {
            if let Some((key, rec)) = parse_line(line) {
                match records.get(&key) {
                    Some(existing) if existing.total_tokens() >= rec.total_tokens() => {}
                    _ => {
                        records.insert(key, rec);
                    }
                }
            }
        }
    }

    let now: DateTime<Local> = Local::now();
    let today = now.date_naive();
    let week_cutoff = now - Duration::days(7);
    let last30_cutoff = now - Duration::days(30);

    let mut report = CostReport {
        updated_at: now.to_rfc3339(),
        empty: records.is_empty(),
        ..Default::default()
    };

    for rec in records.values() {
        let local = rec.ts.with_timezone(&Local);
        let date = local.date_naive();
        let cost = rec.cost();
        let tokens = rec.display_tokens();

        if date == today {
            report.today_usd += cost;
            report.today_tokens += tokens;
        }
        if local >= week_cutoff {
            report.week_usd += cost;
            report.week_tokens += tokens;
        }
        if date.year() == today.year() && date.month() == today.month() {
            report.month_usd += cost;
            report.month_tokens += tokens;
        }
        if local >= last30_cutoff {
            report.last30_usd += cost;
            report.last30_tokens += tokens;
        }
    }

    report
}
