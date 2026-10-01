//! Calcula el costo a partir de los logs locales de Claude Code.
//!
//! Parsea %USERPROFILE%\.claude\projects\**\*.jsonl, igual que ccusage:
//!  - solo lineas con type=="assistant" que traen message.usage
//!  - deduplica por requestId (se queda con el registro mas completo)
//!  - multiplica tokens por el precio del modelo
//!
//! Nota: Claude Code no guarda el costo real, asi que esto es una ESTIMACION.
//! Los tokens mostrados son el total procesado (input + output + escritura y
//! lectura de cache), igual que en Codex, para que ambos se comparen 1 a 1.
//!
//! El escaneo es incremental: cada archivo recuerda hasta que byte se leyo y
//! solo se parsean las lineas nuevas. Antes se releian ~2 GB de logs cada 60s.

use crate::credentials::claude_dir;
use crate::model::{CostReport, ModelUsage};
use crate::pricing;
use chrono::{DateTime, Datelike, Duration, Local, Utc};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

/// Solo miramos archivos modificados en los ultimos N dias (cubre hoy / semana
/// / mes-calendario de 31d / 30d rolling). Acelera mucho el escaneo. Margen
/// extra por desfase de mtime de OneDrive y zonas horarias.
const WINDOW_DAYS: i64 = 35;

#[derive(Clone)]
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
    /// Total procesado, cache incluida (mismo criterio que Codex).
    fn total_tokens(&self) -> u64 {
        self.input + self.output + self.cache_create_5m + self.cache_create_1h + self.cache_read
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

/// Lo ya leido de un .jsonl: offset del ultimo salto de linea completo y los
/// registros que salieron de ahi.
#[derive(Default)]
struct FileState {
    offset: u64,
    modified: Option<SystemTime>,
    records: HashMap<String, Record>,
}

impl FileState {
    fn update(&mut self, path: &std::path::Path, len: u64, modified: Option<SystemTime>) -> std::io::Result<()> {
        // Si el archivo se achico o lo reescribieron, se vuelve a leer entero.
        if len < self.offset || (len == self.offset && modified != self.modified) {
            *self = Self::default();
        }
        if len != self.offset {
            let mut reader = BufReader::new(File::open(path)?);
            reader.seek(SeekFrom::Start(self.offset))?;
            let mut buf = Vec::new();
            loop {
                buf.clear();
                let bytes = reader.read_until(b'\n', &mut buf)?;
                // Linea a medio escribir: se retoma en la proxima vuelta.
                if bytes == 0 || buf.last() != Some(&b'\n') {
                    break;
                }
                self.offset += bytes as u64;
                if let Some((key, rec)) = parse_line(&String::from_utf8_lossy(&buf)) {
                    keep_best(&mut self.records, key, rec);
                }
            }
        }
        self.modified = modified;
        Ok(())
    }
}

/// requestId -> registro mas completo (Claude Code reescribe el mismo request
/// varias veces mientras hace streaming).
fn keep_best(map: &mut HashMap<String, Record>, key: String, rec: Record) {
    match map.get(&key) {
        Some(existing) if existing.total_tokens() >= rec.total_tokens() => {}
        _ => {
            map.insert(key, rec);
        }
    }
}

static FILES: Mutex<Option<HashMap<PathBuf, FileState>>> = Mutex::new(None);

/// Recorre los logs y agrega el costo en ventanas de tiempo.
pub fn compute() -> CostReport {
    let projects = claude_dir().join("projects");
    let cutoff_file = SystemTime::now()
        .checked_sub(std::time::Duration::from_secs((WINDOW_DAYS as u64) * 86_400))
        .unwrap_or(std::time::UNIX_EPOCH);

    let mut guard = FILES.lock().unwrap_or_else(|e| e.into_inner());
    let files = guard.get_or_insert_with(HashMap::new);
    let mut present = HashSet::new();

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
        let Ok(meta) = entry.metadata() else { continue };
        let modified = meta.modified().ok();
        // Salta archivos viejos (fuera de la ventana).
        if modified.is_some_and(|m| m < cutoff_file) {
            continue;
        }
        let path = entry.into_path();
        let state = files.entry(path.clone()).or_default();
        if state.update(&path, meta.len(), modified).is_err() {
            *state = FileState::default();
        }
        present.insert(path);
    }
    files.retain(|p, _| present.contains(p));

    // Dedup global: un mismo requestId puede aparecer en varios archivos
    // (sesiones resumidas o forkeadas).
    let mut records: HashMap<String, Record> = HashMap::new();
    for (key, rec) in files.values().flat_map(|f| &f.records) {
        keep_best(&mut records, key.clone(), rec.clone());
    }
    drop(guard);

    let now: DateTime<Local> = Local::now();
    let today = now.date_naive();
    let week_cutoff = now - Duration::days(7);
    let last30_cutoff = now - Duration::days(30);

    let mut report = CostReport {
        updated_at: now.to_rfc3339(),
        empty: records.is_empty(),
        daily: vec![0.0; 30],
        ..Default::default()
    };
    // Desglose por modelo dentro de la ventana de 30 dias (costo + tokens).
    let mut model_cost: HashMap<String, f64> = HashMap::new();
    let mut model_tokens: HashMap<String, u64> = HashMap::new();

    for rec in records.values() {
        let local = rec.ts.with_timezone(&Local);
        let date = local.date_naive();
        let cost = rec.cost();
        let tokens = rec.total_tokens();

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

        // Histograma diario (ultimos 30 dias) y desglose por modelo (30 dias).
        let days_ago = (today - date).num_days();
        if (0..30).contains(&days_ago) {
            report.daily[(29 - days_ago) as usize] += cost;
        }
        if local >= last30_cutoff {
            *model_cost.entry(rec.model.clone()).or_insert(0.0) += cost;
            *model_tokens.entry(rec.model.clone()).or_insert(0) += tokens;
        }
    }

    // Desglose por modelo ordenado de mayor a menor costo.
    let mut models: Vec<ModelUsage> = model_cost
        .into_iter()
        .map(|(model, cost_usd)| ModelUsage {
            tokens: model_tokens.get(&model).copied().unwrap_or(0),
            model,
            cost_usd,
        })
        .collect();
    models.sort_by(|a, b| b.cost_usd.partial_cmp(&a.cost_usd).unwrap_or(std::cmp::Ordering::Equal));
    report.top_model = models.first().map(|m| m.model.clone()).unwrap_or_default();
    report.models = models;

    report
}
