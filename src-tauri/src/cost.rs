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
//! Lectura eficiente (ver logscan): solo se leen bytes nuevos de cada archivo
//! y el progreso se guarda en disco, asi que reiniciar la app no relee nada.

use crate::credentials::claude_dir;
use crate::logscan::{self, BackgroundIo, SavePolicy};
use crate::model::{CostReport, ModelUsage};
use crate::pricing;
use chrono::{DateTime, Datelike, Duration, Local, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// Solo miramos archivos modificados en los ultimos N dias (cubre hoy / semana
/// / mes-calendario de 31d / 30d rolling). Margen extra por desfase de mtime y
/// zonas horarias.
const WINDOW_DAYS: i64 = 35;
/// Cambia esto si cambia el formato del cache o la forma de parsear: fuerza a
/// reconstruirlo desde cero.
const CACHE_VERSION: u32 = 1;
const CACHE_FILE: &str = "claude-usage-cache.json";

#[derive(Clone, Serialize, Deserialize)]
struct Record {
    /// Epoch en segundos (UTC).
    #[serde(rename = "t")]
    ts: i64,
    #[serde(rename = "m")]
    model: String,
    #[serde(rename = "i")]
    input: u64,
    #[serde(rename = "o")]
    output: u64,
    #[serde(rename = "w5")]
    cache_create_5m: u64,
    #[serde(rename = "w1")]
    cache_create_1h: u64,
    #[serde(rename = "r")]
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

// Solo los campos que importan de una linea; serde salta el resto (el texto
// de los mensajes) sin armar un arbol JSON entero.
#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    uuid: Option<String>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    model: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Usage {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_read_input_tokens: u64,
    cache_creation: Option<CacheCreation>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CacheCreation {
    ephemeral_5m_input_tokens: u64,
    ephemeral_1h_input_tokens: u64,
}

/// Parsea una linea jsonl a Record (o None si no es un assistant con usage).
fn parse_line(line: &str) -> Option<(u64, Record)> {
    // Filtro barato antes de parsear: la gran mayoria de lineas no sirven.
    if !line.contains("\"assistant\"") || !line.contains("\"usage\"") {
        return None;
    }
    let v: Line = serde_json::from_str(line).ok()?;
    if v.kind.as_deref() != Some("assistant") {
        return None;
    }
    let msg = v.message?;
    let usage = msg.usage?;
    let ts = DateTime::parse_from_rfc3339(v.timestamp.as_deref()?).ok()?.timestamp();

    // Desglose de cache_creation si existe; si no cuadra, todo va al bucket de 5m.
    let cc_total = usage.cache_creation_input_tokens;
    let (cc_5m, cc_1h) = match &usage.cache_creation {
        Some(b) if b.ephemeral_5m_input_tokens + b.ephemeral_1h_input_tokens > 0 => {
            (b.ephemeral_5m_input_tokens, b.ephemeral_1h_input_tokens)
        }
        _ => (cc_total, 0),
    };

    // Clave de deduplicacion: requestId; si no, el uuid de la linea.
    let key = v.request_id.or(v.uuid).filter(|k| !k.is_empty())?;
    Some((
        logscan::fnv64(&key),
        Record {
            ts,
            model: msg.model.unwrap_or_else(|| "unknown".into()),
            input: usage.input_tokens,
            output: usage.output_tokens,
            cache_create_5m: cc_5m,
            cache_create_1h: cc_1h,
            cache_read: usage.cache_read_input_tokens,
        },
    ))
}

/// Lo ya leido de un .jsonl: offset del ultimo salto de linea completo y los
/// registros que salieron de ahi.
#[derive(Default, Serialize, Deserialize)]
struct FileState {
    offset: u64,
    mtime: u64,
    records: HashMap<u64, Record>,
}

/// requestId -> registro mas completo (Claude Code reescribe el mismo request
/// varias veces mientras hace streaming).
fn keep_best(map: &mut HashMap<u64, Record>, key: u64, rec: Record) {
    match map.get(&key) {
        Some(existing) if existing.total_tokens() >= rec.total_tokens() => {}
        _ => {
            map.insert(key, rec);
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    version: u32,
    files: HashMap<String, FileState>,
}

#[derive(Default)]
struct Scanner {
    loaded: bool,
    cache: Cache,
    policy: SavePolicy,
}

static SCANNER: Mutex<Option<Scanner>> = Mutex::new(None);

impl Scanner {
    /// Pone al dia el cache con lo nuevo de cada archivo. Devuelve los bytes leidos.
    fn scan(&mut self) -> u64 {
        if !self.loaded {
            self.loaded = true;
            if let Some(c) = logscan::load::<Cache>(CACHE_FILE).filter(|c| c.version == CACHE_VERSION) {
                self.cache = c;
            }
            self.cache.version = CACHE_VERSION;
        }
        let projects = claude_dir().join("projects");
        let now = Utc::now();
        let cutoff_ts = (now - Duration::days(WINDOW_DAYS)).timestamp();
        let cutoff_mtime = (cutoff_ts.max(0) as u64) * 1_000_000_000;
        let files = &mut self.cache.files;
        let mut present = HashSet::new();
        let mut bytes_read = 0;

        for entry in walkdir::WalkDir::new(&projects).into_iter().flatten() {
            if !entry.file_type().is_file() || entry.path().extension().map_or(true, |e| e != "jsonl") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let (len, mtime) = (meta.len(), logscan::mtime_nanos(&meta));
            // Archivos sin tocar desde antes de la ventana: no pueden aportar.
            if mtime < cutoff_mtime {
                continue;
            }
            let key = entry.path().to_string_lossy().into_owned();
            let state = files.entry(key.clone()).or_default();
            present.insert(key);
            if len == state.offset && mtime == state.mtime {
                continue; // sin cambios: ni se abre
            }
            // Si se achico o lo reescribieron con el mismo tamaño, se relee entero.
            if len <= state.offset {
                *state = FileState::default();
            }
            let start = state.offset;
            let records = &mut state.records;
            match logscan::read_new_lines(entry.path(), start, |line| {
                if let Some((k, rec)) = parse_line(line) {
                    keep_best(records, k, rec);
                }
            }) {
                Ok(offset) => {
                    bytes_read += offset - start;
                    state.offset = offset;
                    state.mtime = mtime;
                }
                Err(_) => *state = FileState::default(),
            }
            self.policy.mark_dirty();
        }

        let before = files.len();
        files.retain(|p, _| present.contains(p));
        for state in files.values_mut() {
            let n = state.records.len();
            state.records.retain(|_, r| r.ts >= cutoff_ts);
            if state.records.len() != n {
                self.policy.mark_dirty();
            }
        }
        if files.len() != before {
            self.policy.mark_dirty();
        }
        bytes_read
    }

    fn save_if_needed(&mut self, bytes_read: u64, force: bool) {
        if self.policy.should_save(bytes_read, force) {
            logscan::save(CACHE_FILE, &self.cache);
            self.policy.saved();
        }
    }
}

/// Guarda el progreso pendiente (al cerrar la app).
pub fn flush() {
    if let Ok(mut guard) = SCANNER.try_lock() {
        if let Some(s) = guard.as_mut() {
            s.save_if_needed(0, true);
        }
    }
}

/// Recorre los logs y agrega el costo en ventanas de tiempo.
pub fn compute() -> CostReport {
    let _bg = BackgroundIo::begin();
    let mut guard = SCANNER.lock().unwrap_or_else(|e| e.into_inner());
    let scanner = guard.get_or_insert_with(Scanner::default);
    let bytes_read = scanner.scan();
    scanner.save_if_needed(bytes_read, false);

    // Dedup global: un mismo requestId puede aparecer en varios archivos
    // (sesiones resumidas o forkeadas).
    let mut records: HashMap<u64, Record> = HashMap::new();
    for (key, rec) in scanner.cache.files.values().flat_map(|f| &f.records) {
        keep_best(&mut records, *key, rec.clone());
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
        let Some(local) = Local.timestamp_opt(rec.ts, 0).single() else { continue };
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
