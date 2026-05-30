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
//! load n1 0.002             # current drawn out of a node (amps)
//! emlimit met5 0.01         # per-layer EM current limit (amps/segment)
//! ```
//!
//! Pure std — fully unit-tested offline.

use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Resistor {
    pub a: String,
    pub b: String,
    pub r: f64,
    pub layer: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PdnSpec {
    pub vdd: f64,
    pub pads: Vec<(String, f64)>,
    pub resistors: Vec<Resistor>,
    pub loads: Vec<(String, f64)>,
    pub em_limits: BTreeMap<String, f64>,
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
