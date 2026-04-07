//! Cost ledger — append-only JSONL of every AI call's spend.
//!
//! Phase 9 records one [`CostEntry`] per `/ai/chat` call to a
//! separate file (`%LOCALAPPDATA%/xlpod/cost.jsonl`) so a user
//! can share an audit log for debugging without leaking spend
//! information. The launcher also keeps an in-memory rollup
//! (today only) for the tray badge and the `/ai/cost/today` route;
//! it is rebuilt at startup by replaying today's lines from the
//! ledger.
//!
//! All math is in integer micro-USD (`u64`) so a long session
//! cannot accumulate float drift.
//!
//! Pricing tables are static for now; a future revision will let
//! the user override them via `XLPOD_PRICE_TABLE_PATH` so a price
//! change at the provider does not require a launcher rebuild.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::Mutex};

use crate::ai::types::Usage;

/// Default daily cap in micro-USD ($5/day).
pub const DEFAULT_DAILY_BUDGET_MICROS: u64 = 5_000_000;

#[derive(Debug, Clone, Copy)]
pub struct PriceTable {
    /// Micro-USD per million input tokens.
    pub input_micros_per_mtok: u64,
    /// Micro-USD per million output tokens.
    pub output_micros_per_mtok: u64,
}

/// Built-in price table. Update when providers change pricing.
fn price_for(provider: &str, model: &str) -> PriceTable {
    match (provider, model) {
        ("anthropic", m) if m.starts_with("claude-opus-4") => PriceTable {
            input_micros_per_mtok: 15_000_000,  // $15/M input
            output_micros_per_mtok: 75_000_000, // $75/M output
        },
        ("anthropic", m) if m.starts_with("claude-sonnet-4") => PriceTable {
            input_micros_per_mtok: 3_000_000,
            output_micros_per_mtok: 15_000_000,
        },
        ("anthropic", _) => PriceTable {
            input_micros_per_mtok: 3_000_000,
            output_micros_per_mtok: 15_000_000,
        },
        _ => PriceTable {
            input_micros_per_mtok: 0,
            output_micros_per_mtok: 0,
        },
    }
}

pub fn cost_for(provider: &str, model: &str, usage: &Usage) -> u64 {
    let p = price_for(provider, model);
    let input_micros = usage.input_tokens.saturating_mul(p.input_micros_per_mtok) / 1_000_000;
    let output_micros = usage.output_tokens.saturating_mul(p.output_micros_per_mtok) / 1_000_000;
    input_micros.saturating_add(output_micros)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostEntry {
    pub ts_ms: u128,
    pub provider: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub usd_micros: u64,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct CostRollup {
    pub date: String,
    pub total_usd_micros: u64,
    pub by_model: Vec<CostEntry>,
}

#[derive(Debug, Clone)]
pub struct CostLedger {
    inner: Arc<CostInner>,
}

#[derive(Debug)]
struct CostInner {
    path: PathBuf,
    file: Mutex<Option<tokio::fs::File>>,
    today: std::sync::RwLock<HashMap<(String, String), CostEntry>>,
    daily_cap_micros: u64,
}

impl CostLedger {
    pub async fn open(path: PathBuf, daily_cap_micros: u64) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;
        Ok(Self {
            inner: Arc::new(CostInner {
                path,
                file: Mutex::new(Some(file)),
                today: std::sync::RwLock::new(HashMap::new()),
                daily_cap_micros,
            }),
        })
    }

    pub fn daily_cap_micros(&self) -> u64 {
        self.inner.daily_cap_micros
    }

    pub async fn record(
        &self,
        provider: &str,
        model: &str,
        usage: &Usage,
    ) -> Result<u64, std::io::Error> {
        let usd_micros = cost_for(provider, model, usage);
        let entry = CostEntry {
            ts_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
            provider: provider.to_string(),
            model: model.to_string(),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            usd_micros,
        };
        if let Ok(mut line) = serde_json::to_vec(&entry) {
            line.push(b'\n');
            let mut guard = self.inner.file.lock().await;
            if let Some(file) = guard.as_mut() {
                let _ = file.write_all(&line).await;
                let _ = file.flush().await;
            }
        }
        let mut today = self
            .inner
            .today
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = (provider.to_string(), model.to_string());
        let row = today.entry(key).or_insert_with(|| CostEntry {
            ts_ms: entry.ts_ms,
            provider: entry.provider.clone(),
            model: entry.model.clone(),
            input_tokens: 0,
            output_tokens: 0,
            usd_micros: 0,
        });
        row.input_tokens = row.input_tokens.saturating_add(usage.input_tokens);
        row.output_tokens = row.output_tokens.saturating_add(usage.output_tokens);
        row.usd_micros = row.usd_micros.saturating_add(usd_micros);
        Ok(usd_micros)
    }

    pub fn today_total_micros(&self) -> u64 {
        let today = self
            .inner
            .today
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        today.values().map(|e| e.usd_micros).sum()
    }

    pub fn over_budget(&self) -> bool {
        self.daily_cap_micros() > 0 && self.today_total_micros() >= self.daily_cap_micros()
    }

    pub fn rollup(&self) -> CostRollup {
        let today = self
            .inner
            .today
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let by_model: Vec<CostEntry> = today.values().cloned().collect();
        CostRollup {
            date: format_today_utc(),
            total_usd_micros: by_model.iter().map(|e| e.usd_micros).sum(),
            by_model,
        }
    }

    #[allow(dead_code)]
    pub fn path(&self) -> &PathBuf {
        &self.inner.path
    }
}

fn format_today_utc() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        now.month() as u8,
        now.day()
    )
}
