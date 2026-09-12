//! Local Codex token accounting. Only structured usage events are retained.
//! API-equivalent estimates, never subscription charges or account-wide spend.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, Utc};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub model: String,
    pub tokens: u64,
    pub cost_usd: Option<f64>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CostReport {
    pub today_usd: f64,
    pub last30_usd: f64,
    pub month_tokens: u64,
    pub week_tokens: u64,
    pub last30_tokens: u64,
    pub daily: Vec<f64>,
    pub models: Vec<ModelUsage>,
    pub unpriced_tokens: u64,
    pub updated_at: String,
    pub empty: bool,
    pub incomplete: bool,
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Tokens {
    input: u64,
    cached: u64,
    write: u64,
    output: u64,
}

impl Tokens {
    fn parse(v: &Value) -> Self {
        let n = |key| v.get(key).and_then(Value::as_u64).unwrap_or(0);
        Self {
            input: n("input_tokens"),
            cached: n("cached_input_tokens"),
            write: n("cache_write_input_tokens"),
            output: n("output_tokens"),
        }
    }

    fn total(self) -> u64 {
        // Cached input is included in input; reasoning is included in output.
        self.input + self.output
    }

    fn delta(self, previous: Self) -> Self {
        Self {
            input: self.input.saturating_sub(previous.input),
            cached: self.cached.saturating_sub(previous.cached),
            write: self.write.saturating_sub(previous.write),
            output: self.output.saturating_sub(previous.output),
        }
    }
}

#[derive(Clone)]
struct Record {
    key: String,
    date: NaiveDate,
    model: String,
    tokens: Tokens,
}

#[derive(Default)]
struct Rollout {
    offset: u64,
    modified: Option<SystemTime>,
    model: String,
    previous: Tokens,
    records: Vec<Record>,
}

impl Rollout {
    fn ingest(&mut self, line: &str, cutoff: NaiveDate) {
        if !line.contains("\"token_count\"") && !line.contains("\"turn_context\"") {
            return;
        }
        let Ok(event) = serde_json::from_str::<Value>(line) else { return };
        let p = &event["payload"];
        if event["type"] == "turn_context" {
            if let Some(model) = p["model"].as_str() {
                self.model = model.to_string();
            }
            return;
        }
        if event["type"] != "event_msg" || p["type"] != "token_count" {
            return;
        }
        let Some(timestamp) = event["timestamp"].as_str() else { return };
        let Ok(ts) = DateTime::parse_from_rfc3339(timestamp) else { return };
        let cumulative = p.pointer("/info/total_token_usage").or_else(|| p.get("thread_token_usage"));
        let (tokens, identity) = if let Some(total) = cumulative.filter(|v| v.is_object()) {
            let current = Tokens::parse(total);
            let delta = if current.input < self.previous.input || current.output < self.previous.output {
                // A resumed/compacted thread can reset its cumulative counters.
                p.pointer("/info/last_token_usage").or_else(|| p.get("usage"))
                    .map(Tokens::parse).unwrap_or_default()
            } else {
                current.delta(self.previous)
            };
            self.previous = current;
            (delta, current)
        } else if let Some(usage) = p.get("usage").filter(|v| v.is_object()) {
            let tokens = Tokens::parse(usage);
            (tokens, tokens)
        } else {
            return;
        };
        let date = ts.with_timezone(&Local).date_naive();
        if tokens.total() == 0 || date < cutoff { return; }
        // Forked rollouts contain copied parent events. Timestamp + cumulative
        // counters identify the original event across files, independently of model.
        let key = p["response_id"].as_str().filter(|s| !s.is_empty()).map(str::to_owned)
            .unwrap_or_else(|| format!("{}:{}:{}:{}:{}", timestamp, identity.input,
                identity.cached, identity.write, identity.output));
        let model = p["model"].as_str().unwrap_or(&self.model);
        self.records.push(Record {
            key, date, tokens,
            model: if model.is_empty() { "unknown" } else { model }.to_string(),
        });
    }

    fn update(&mut self, path: &Path, cutoff: NaiveDate) -> std::io::Result<()> {
        let file = File::open(path)?;
        let meta = file.metadata()?;
        let modified = meta.modified().ok();
        if meta.len() < self.offset || (meta.len() == self.offset && modified != self.modified) {
            *self = Self::default();
        }
        if meta.len() != self.offset {
            let mut reader = BufReader::new(file);
            reader.seek(SeekFrom::Start(self.offset))?;
            let mut line = String::new();
            loop {
                line.clear();
                let bytes = reader.read_line(&mut line)?;
                if bytes == 0 || !line.ends_with('\n') { break; }
                self.offset += bytes as u64;
                self.ingest(&line, cutoff);
            }
        }
        self.modified = modified;
        self.records.retain(|r| r.date >= cutoff);
        Ok(())
    }
}

#[derive(Default)]
pub struct CostCache {
    files: HashMap<PathBuf, Rollout>,
}

impl CostCache {
    pub fn compute(&mut self, home: &Path) -> CostReport {
        let now = Local::now();
        let today = now.date_naive();
        let cutoff = today - Duration::days(35);
        let mut present = HashSet::new();
        let mut incomplete = false;
        for dir in ["sessions", "archived_sessions"] {
            let root = home.join(dir);
            if !root.exists() { continue; }
            for entry in WalkDir::new(root) {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => { incomplete = true; continue; }
                };
                if !entry.file_type().is_file() || entry.path().extension().and_then(|s| s.to_str()) != Some("jsonl") {
                    continue;
                }
                let Ok(meta) = entry.metadata() else { incomplete = true; continue };
                if let Ok(modified) = meta.modified() {
                    if DateTime::<Utc>::from(modified).with_timezone(&Local).date_naive() < cutoff {
                        continue;
                    }
                }
                let path = entry.into_path();
                if self.files.entry(path.clone()).or_default().update(&path, cutoff).is_err() {
                    incomplete = true;
                }
                present.insert(path);
            }
        }
        self.files.retain(|p, _| present.contains(p));
        let mut unique: HashMap<&str, &Record> = HashMap::new();
        for r in self.files.values().flat_map(|file| &file.records) {
            unique.entry(&r.key).and_modify(|old| {
                if r.tokens.total() > old.tokens.total() { *old = r; }
            }).or_insert(r);
        }
        let mut report = CostReport {
            daily: vec![0.0; 30], updated_at: now.to_rfc3339(), empty: true,
            incomplete, ..Default::default()
        };
        let mut models: HashMap<String, ModelUsage> = HashMap::new();
        for r in unique.values() {
            let days_ago = (today - r.date).num_days();
            if days_ago < 0 { continue; }
            let tokens = r.tokens.total();
            let cost = estimate(&r.model, r.tokens);
            if r.date.year() == today.year() && r.date.month() == today.month() {
                report.month_tokens += tokens;
            }
            if days_ago < 7 { report.week_tokens += tokens; }
            if days_ago >= 30 { continue; }
            report.empty = false;
            report.last30_tokens += tokens;
            if let Some(cost) = cost {
                if days_ago == 0 { report.today_usd += cost; }
                report.last30_usd += cost;
                report.daily[(29 - days_ago) as usize] += cost;
            } else {
                report.unpriced_tokens += tokens;
            }
            let model = models.entry(r.model.clone()).or_insert_with(|| ModelUsage {
                model: r.model.clone(), tokens: 0, cost_usd: cost.map(|_| 0.0),
            });
            model.tokens += tokens;
            if let (Some(total), Some(cost)) = (&mut model.cost_usd, cost) { *total += cost; }
        }
        report.models = models.into_values().collect();
        report.models.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.model.cmp(&b.model)));
        report
    }
}

fn estimate(model: &str, tokens: Tokens) -> Option<f64> {
    // Standard, short-context USD / 1M, verified 2026-09-12:
    // https://developers.openai.com/api/docs/pricing
    // Deliberately no guessed price for unknown models. This baseline estimate
    // excludes service-tier / long-context uplifts and non-token tools.
    let model = model.to_ascii_lowercase();
    let model = model.strip_suffix("-latest").unwrap_or(&model);
    let model = if model.len() > 11 && model.as_bytes()[model.len() - 11] == b'-'
        && model[model.len() - 10..].chars().all(|c| c.is_ascii_digit() || c == '-') {
        &model[..model.len() - 11]
    } else { model };
    let (input, cached, write, output) = match model {
        "gpt-6-astra" => (10.0, 1.0, 12.5, 50.0),
        "gpt-5.6-sol" => (4.0, 0.4, 5.0, 20.0),
        "gpt-5.6-terra" => (2.0, 0.2, 2.5, 12.0),
        "gpt-5.6-luna" => (0.2, 0.02, 0.25, 1.2),
        "gpt-5.5" => (5.0, 0.5, 5.0, 30.0),
        "gpt-5.5-pro" | "gpt-5.4-pro" => (30.0, 30.0, 30.0, 180.0),
        "gpt-5.4" => (2.5, 0.25, 2.5, 15.0),
        "gpt-5.4-mini" => (0.75, 0.075, 0.75, 4.5),
        "gpt-5.4-nano" => (0.2, 0.02, 0.2, 1.25),
        "gpt-5.3-codex" | "gpt-5.2" => (1.75, 0.175, 1.75, 14.0),
        "gpt-5.1" | "gpt-5" => (1.25, 0.125, 1.25, 10.0),
        "gpt-5-mini" => (0.25, 0.025, 0.25, 2.0),
        "gpt-5-nano" => (0.05, 0.005, 0.05, 0.4),
        _ => return None,
    };
    let cached_tokens = tokens.cached.min(tokens.input);
    let write_tokens = tokens.write.min(tokens.input - cached_tokens);
    Some(((tokens.input - cached_tokens - write_tokens) as f64 * input
        + cached_tokens as f64 * cached + write_tokens as f64 * write
        + tokens.output as f64 * output) / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn incremental_scan_counts_archives_once_and_waits_for_complete_lines() {
        let temp = std::env::temp_dir().join(format!("claudebar-codex-{}-{}", std::process::id(),
            SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(temp.join("sessions")).unwrap();
        std::fs::create_dir_all(temp.join("archived_sessions")).unwrap();
        let stamp = Local::now().to_rfc3339();
        let context = "{\"type\":\"turn_context\",\"payload\":{\"model\":\"gpt-5.6-sol\"}}\n";
        let event = serde_json::json!({"timestamp": stamp, "type":"event_msg", "payload":{
            "type":"token_count", "info":{"total_token_usage":{"input_tokens":1000,"output_tokens":100}}
        }}).to_string();
        let file = temp.join("sessions/rollout-one.jsonl");
        let archive = temp.join("archived_sessions/rollout-copy.jsonl");
        std::fs::write(&file, format!("{context}{event}\n")).unwrap();
        std::fs::write(&archive, format!("{context}{event}\n")).unwrap();
        let mut cache = CostCache::default();
        let first = cache.compute(&temp);
        assert_eq!(first.last30_tokens, 1100);
        assert_eq!(first.models.len(), 1);
        assert_eq!(first.daily.len(), 30);
        assert_eq!(cache.compute(&temp).last30_tokens, 1100);
        let direct = serde_json::json!({"timestamp":stamp,"type":"event_msg","payload":{
            "type":"token_count","response_id":"new-response","usage":{"input_tokens":200,"output_tokens":20}
        }}).to_string();
        let mut append = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        write!(append, "{direct}").unwrap();
        assert_eq!(cache.compute(&temp).last30_tokens, 1100);
        writeln!(append).unwrap();
        drop(append);
        assert_eq!(cache.compute(&temp).last30_tokens, 1320);
        std::fs::remove_file(file).unwrap();
        assert_eq!(cache.compute(&temp).last30_tokens, 1100);
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn cached_and_reasoning_tokens_are_not_counted_twice() {
        let t = Tokens::parse(&serde_json::json!({"input_tokens":1000,"cached_input_tokens":800,
            "output_tokens":100,"reasoning_output_tokens":60,"total_tokens":1100}));
        assert_eq!(t.total(), 1100);
        assert!((estimate("gpt-5.6-sol", t).unwrap() - 0.00312).abs() < 1e-9);
        assert!(estimate("unpublished-model", t).is_none());
    }

    #[test]
    fn cumulative_events_deduplicate_and_follow_model_switches() {
        let mut rollout = Rollout::default();
        let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        rollout.ingest(r#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}"#, cutoff);
        let event = r#"{"timestamp":"2026-09-12T12:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"output_tokens":100}}}}"#;
        rollout.ingest(event, cutoff);
        rollout.ingest(event, cutoff);
        rollout.ingest(r#"{"type":"turn_context","payload":{"model":"gpt-5.6-luna"}}"#, cutoff);
        rollout.ingest(r#"{"timestamp":"2026-09-12T12:01:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1600,"output_tokens":150}}}}"#, cutoff);
        assert_eq!(rollout.records.len(), 2);
        assert_eq!(rollout.records[0].tokens.total(), 1100);
        assert_eq!(rollout.records[1].tokens.total(), 650);
        assert_eq!(rollout.records[1].model, "gpt-5.6-luna");
        let mut fork = Rollout::default();
        fork.ingest(event, cutoff);
        assert_eq!(fork.records[0].key, rollout.records[0].key);
    }

    #[test]
    fn counter_reset_uses_last_request_and_direct_usage_is_supported() {
        let mut rollout = Rollout { previous: Tokens { input: 5000, ..Default::default() }, ..Default::default() };
        let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        rollout.ingest(r#"{"timestamp":"2026-09-12T12:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000},"last_token_usage":{"input_tokens":200,"output_tokens":10}}}}"#, cutoff);
        rollout.ingest(r#"{"timestamp":"2026-09-12T12:01:00Z","type":"event_msg","payload":{"type":"token_count","response_id":"response-1","usage":{"input_tokens":100,"output_tokens":20}}}"#, cutoff);
        assert_eq!(rollout.records[0].tokens.total(), 210);
        assert_eq!(rollout.records[1].tokens.total(), 120);
    }
}
