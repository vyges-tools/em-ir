//! EM/IR job: the declarative description of what to analyze.
//!
//! An `.emir` job is a tiny `key: value` file (std-only parser — no deps):
//!
//! ```text
//! design:        top
//! pdn:           top.pdn        # the PDN resistor network
//! ir_limit_pct:  5.0            # fail threshold: max allowed IR drop (% of vdd)
//! ```

use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct EmIrJob {
    pub design: String,
    pub pdn: String, // a described `.pdn` network (empty when extracting from DEF/LEF)
    pub ir_limit_pct: f64,
    // PDN extraction from layout: a DEF (special-net power grid) + tech LEF (layer
    // sheet resistance) build the resistor network instead of a hand-written `.pdn`.
    pub def: String,
    pub lef: String,
    pub vdd: f64,           // supply voltage for the extracted grid
    pub pad_layer: String,  // metal layer whose nodes are tied to the pads (e.g. top metal)
    pub via_res: f64,       // per-via resistance (ohms) bridging layers at a via point
    pub total_current: f64, // total static current (A), spread over the load-layer nodes
    // The char -> em-ir seam on a real layout: per-instance current from DEF
    // COMPONENTS + a char-derived cell -> switching-energy map. Each instance's
    // current lands on the nearest grid node; the same energy drives a switch event
    // for dynamic IR. `power_map` empty -> fall back to `total_current` / `.pdn`.
    pub power_map: String,  // cell -> switching energy (pJ) [+ leakage nW], from char
    pub decap_map: String,  // decap cell -> capacitance (pF); placed decap from the DEF
    pub clock_ghz: f64,     // switching frequency (GHz) for the average current
    pub activity: f64,      // switching activity factor (0..1)
    pub switch_t_ns: f64,   // dynamic: the (worst-case simultaneous) switch time
    pub switch_dur_ns: f64, // dynamic: per-switch transition duration
    pub node_cap_pf: f64,   // optional uniform decap per load node (pF)
    pub base_dir: String,
}

#[derive(Debug)]
pub struct JobError(pub String);
impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "job error: {}", self.0)
    }
}
impl std::error::Error for JobError {}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    }
}

impl EmIrJob {
    pub fn parse(text: &str, base_dir: &str) -> Result<EmIrJob, JobError> {
        let mut kv: BTreeMap<String, String> = BTreeMap::new();
        for raw in text.lines() {
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            let (k, v) = line
                .split_once(':')
                .ok_or_else(|| JobError(format!("expected 'key: value', got {line:?}")))?;
            kv.insert(k.trim().to_lowercase(), v.trim().to_string());
        }
        let get = |k: &str| kv.get(k).cloned().ok_or_else(|| JobError(format!("missing key: {k}")));
        let def = kv.get("def").cloned().unwrap_or_default();
        let job = EmIrJob {
            design: get("design")?,
            pdn: kv.get("pdn").cloned().unwrap_or_default(),
            ir_limit_pct: kv.get("ir_limit_pct").and_then(|s| s.parse().ok()).unwrap_or(5.0),
            lef: kv.get("lef").cloned().unwrap_or_default(),
            vdd: kv.get("vdd").and_then(|s| s.parse().ok()).unwrap_or(1.8),
            pad_layer: kv.get("pad_layer").cloned().unwrap_or_default(),
            via_res: kv.get("via_res").and_then(|s| s.parse().ok()).unwrap_or(5.0),
            total_current: kv.get("total_current").and_then(|s| s.parse().ok()).unwrap_or(0.0),
            power_map: kv.get("power_map").cloned().unwrap_or_default(),
            decap_map: kv.get("decap_map").cloned().unwrap_or_default(),
            clock_ghz: kv.get("clock_ghz").and_then(|s| s.parse().ok()).unwrap_or(1.0),
            activity: kv.get("activity").and_then(|s| s.parse().ok()).unwrap_or(0.2),
            switch_t_ns: kv.get("switch_t_ns").and_then(|s| s.parse().ok()).unwrap_or(1.0),
            switch_dur_ns: kv.get("switch_dur_ns").and_then(|s| s.parse().ok()).unwrap_or(0.1),
            node_cap_pf: kv.get("node_cap_pf").and_then(|s| s.parse().ok()).unwrap_or(0.0),
            def,
            base_dir: base_dir.to_string(),
        };
        // Either a described `.pdn` or a DEF+LEF extraction is required.
        if job.pdn.is_empty() && job.def.is_empty() {
            return Err(JobError("either `pdn` or `def` (+`lef`) is required".into()));
        }
        if !job.def.is_empty() {
            if job.lef.is_empty() {
                return Err(JobError("`def` extraction also needs `lef` (layer resistances)".into()));
            }
            if job.pad_layer.is_empty() {
                return Err(JobError("`def` extraction needs `pad_layer` (the supply/top layer)".into()));
            }
        }
        Ok(job)
    }

    pub fn load(path: &str) -> Result<EmIrJob, JobError> {
        let text = std::fs::read_to_string(path).map_err(|e| JobError(format!("{path}: {e}")))?;
        let base = Path::new(path).parent().and_then(|p| p.to_str()).unwrap_or(".");
        EmIrJob::parse(&text, base)
    }

    pub fn resolve(&self, rel: &str) -> String {
        if Path::new(rel).is_absolute() || self.base_dir.is_empty() {
            rel.to_string()
        } else {
            Path::new(&self.base_dir).join(rel).to_string_lossy().into_owned()
        }
    }
}
