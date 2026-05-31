//! Power-distribution-network reader — the resistor mesh the solver needs.
//!
//! A `.pdn` file is a small whitespace-keyword format (std-only — no deps):
//!
//! ```text
//! vdd 1.8                   # nominal supply voltage
//! pad p1                    # supply pad, tied to vdd
//! pad p2 1.8                # ...or an explicit voltage
//! res p1 n1 0.05 met5       # resistor: nodeA nodeB ohms [layer]
//! via n1 m1 2.0             # a via resistance (layer = "via")
//! load n1 0.002             # static current drawn out of a node (amps)
//! emlimit met5 0.01         # per-layer EM current limit (amps/segment)
//! ```
//!
//! Dynamic (transient) IR additionally needs node capacitance and switching
//! current events — the latter fed by `vyges-char`'s `internal_power`:
//!
//! ```text
//! cap  n1 0.5               # decoupling/parasitic capacitance at a node (pF)
//! switch n1 0.012 1.0 0.08  # switching event: node energy(pJ) t50(ns) [dur(ns)]
//! ```
//!
//! A `switch` event draws charge `Q = energy/vdd` from the rail as a triangular
//! current pulse peaking at `t50` over `dur`. The `energy` is the per-event supply
//! energy from char (its `internal_power` value, plus the net's load-charging
//! ½·C·V² for a rising output) — this is the char → em-ir seam.
//!
//! Pure std — fully unit-tested offline.

use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Resistor {
    pub a: String,
    pub b: String,
    pub r: f64,
    pub layer: Option<String>,
    /// Per-segment EM current limits (A) — from LEF current-densities × this wire's
    /// width. `em_limit` is the DC-average (Iavg) limit (`None` falls back to the
    /// PDN's flat per-layer `emlimit`); `em_rms_limit`/`em_peak_limit` are the AC
    /// RMS/peak limits checked against the transient current waveform.
    pub em_limit: Option<f64>,
    pub em_rms_limit: Option<f64>,
    pub em_peak_limit: Option<f64>,
}

/// A switching-current event at a node: the rail delivers `energy_pj`/vdd of charge
/// as a triangular pulse peaking at `t50_ns` over `dur_ns`. `energy_pj` is char's
/// per-event supply energy (internal_power + load-charging).
#[derive(Debug, Clone)]
pub struct Switch {
    pub node: String,
    pub energy_pj: f64,
    pub t50_ns: f64,
    pub dur_ns: f64,
}

#[derive(Debug, Clone, Default)]
pub struct PdnSpec {
    pub vdd: f64,
    pub pads: Vec<(String, f64)>,
    pub resistors: Vec<Resistor>,
    pub loads: Vec<(String, f64)>,
    pub em_limits: BTreeMap<String, f64>,
    pub caps: Vec<(String, f64)>, // node decap/parasitic capacitance (pF)
    pub switches: Vec<Switch>,    // switching-current events (dynamic IR)
}

impl PdnSpec {
    /// True when the spec carries dynamic stimulus (switching events) — the engine
    /// then runs the transient IR analysis on top of the static solve.
    pub fn is_dynamic(&self) -> bool {
        !self.switches.is_empty()
    }
}

#[derive(Debug)]
pub struct PdnError(pub String);
impl std::fmt::Display for PdnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "pdn error: {}", self.0)
    }
}
impl std::error::Error for PdnError {}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    }
}

fn num(tok: Option<&str>, what: &str) -> Result<f64, PdnError> {
    tok.and_then(|t| t.parse::<f64>().ok())
        .ok_or_else(|| PdnError(format!("{what}: expected a number")))
}

impl PdnSpec {
    pub fn parse(text: &str) -> Result<PdnSpec, PdnError> {
        let mut spec = PdnSpec { vdd: 0.0, ..Default::default() };
        let mut pads_raw: Vec<(String, Option<f64>)> = Vec::new();
        for raw in text.lines() {
            let toks: Vec<&str> = strip_comment(raw).split_whitespace().collect();
            if toks.is_empty() {
                continue;
            }
            match toks[0] {
                "vdd" => spec.vdd = num(toks.get(1).copied(), "vdd")?,
                "pad" => {
                    let node = toks.get(1).ok_or_else(|| PdnError("pad needs a node".into()))?;
                    let v = match toks.get(2) {
                        Some(t) => Some(num(Some(t), "pad voltage")?),
                        None => None,
                    };
                    pads_raw.push((node.to_string(), v));
                }
                "res" | "via" => {
                    let a = toks.get(1).ok_or_else(|| PdnError("res needs node a".into()))?;
                    let b = toks.get(2).ok_or_else(|| PdnError("res needs node b".into()))?;
                    let r = num(toks.get(3).copied(), "res ohms")?;
                    if r <= 0.0 {
                        return Err(PdnError(format!("res {a}-{b}: resistance must be > 0")));
                    }
                    let layer = if toks[0] == "via" {
                        Some("via".to_string())
                    } else {
                        toks.get(4).map(|s| s.to_string())
                    };
                    spec.resistors.push(Resistor {
                        a: a.to_string(),
                        b: b.to_string(),
                        r,
                        layer,
                        em_limit: None, // .pdn uses the flat per-layer `emlimit`
                        em_rms_limit: None,
                        em_peak_limit: None,
                    });
                }
                "load" => {
                    let node = toks.get(1).ok_or_else(|| PdnError("load needs a node".into()))?;
                    let i = num(toks.get(2).copied(), "load amps")?;
                    spec.loads.push((node.to_string(), i));
                }
                "emlimit" => {
                    let layer = toks.get(1).ok_or_else(|| PdnError("emlimit needs a layer".into()))?;
                    let lim = num(toks.get(2).copied(), "emlimit amps")?;
                    spec.em_limits.insert(layer.to_string(), lim);
                }
                "cap" => {
                    let node = toks.get(1).ok_or_else(|| PdnError("cap needs a node".into()))?;
                    let c = num(toks.get(2).copied(), "cap pF")?;
                    spec.caps.push((node.to_string(), c));
                }
                "switch" => {
                    let node = toks.get(1).ok_or_else(|| PdnError("switch needs a node".into()))?;
                    let energy = num(toks.get(2).copied(), "switch energy(pJ)")?;
                    let t50 = num(toks.get(3).copied(), "switch t50(ns)")?;
                    let dur = match toks.get(4) {
                        Some(t) => num(Some(t), "switch dur(ns)")?,
                        None => 0.1,
                    };
                    if dur <= 0.0 {
                        return Err(PdnError(format!("switch {node}: dur must be > 0")));
                    }
                    spec.switches.push(Switch {
                        node: node.to_string(),
                        energy_pj: energy,
                        t50_ns: t50,
                        dur_ns: dur,
                    });
                }
                other => return Err(PdnError(format!("unknown keyword: {other:?}"))),
            }
        }
        if spec.vdd <= 0.0 {
            return Err(PdnError("vdd must be set and > 0".into()));
        }
        if pads_raw.is_empty() {
            return Err(PdnError("at least one pad is required".into()));
        }
        if spec.resistors.is_empty() {
            return Err(PdnError("at least one resistor is required".into()));
        }
        spec.pads = pads_raw.into_iter().map(|(n, v)| (n, v.unwrap_or(spec.vdd))).collect();
        Ok(spec)
    }

    pub fn load(path: &str) -> Result<PdnSpec, PdnError> {
        let text = std::fs::read_to_string(path).map_err(|e| PdnError(format!("{path}: {e}")))?;
        PdnSpec::parse(&text)
    }
}
