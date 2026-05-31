//! Minimal tech-LEF reader — the per-layer resistances PDN extraction needs.
//!
//! Only the routing-layer electrical attributes are parsed (the geometry of the
//! grid comes from the DEF):
//!
//! ```text
//! LAYER met5
//!   TYPE ROUTING ;
//!   RESISTANCE RPERSQ 0.0285 ;   # sheet resistance, ohm/square
//!   WIDTH 1.6 ;                   # default routing width, microns
//! END met5
//! ```
//!
//! Pure std — fully unit-tested offline.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct LayerR {
    pub rpersq: f64,    // sheet resistance (ohm/square)
    pub width_um: f64,  // default routing width (microns); 0 if unspecified
    pub dc_jmax: f64,   // DC average current-density limit (mA/um); 0 if unspecified
    pub ac_rms: f64,    // AC RMS current-density limit (mA/um); 0 if unspecified
    pub ac_peak: f64,   // AC peak current-density limit (mA/um); 0 if unspecified
}

#[derive(Debug, Clone, Default)]
pub struct TechLef {
    pub layers: BTreeMap<String, LayerR>,
}

#[derive(Debug)]
pub struct LefError(pub String);
impl std::fmt::Display for LefError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "lef error: {}", self.0)
    }
}
impl std::error::Error for LefError {}

impl TechLef {
    pub fn parse(text: &str) -> Result<TechLef, LefError> {
        let mut layers = BTreeMap::new();
        let mut cur: Option<(String, LayerR)> = None;
        for raw in text.lines() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let toks: Vec<&str> = line.split_whitespace().collect();
            match toks.as_slice() {
                ["LAYER", name, ..] => cur = Some((name.to_string(), LayerR::default())),
                // `END <layer>` closes the current layer block
                ["END", name] if cur.as_ref().map(|(n, _)| n == name).unwrap_or(false) => {
                    if let Some((n, l)) = cur.take() {
                        layers.insert(n, l);
                    }
                }
                _ => {
                    if let Some((_, l)) = cur.as_mut() {
                        // RESISTANCE RPERSQ <v> ;   (routing-layer sheet resistance)
                        if toks.len() >= 3 && toks[0] == "RESISTANCE" && toks[1] == "RPERSQ" {
                            l.rpersq = toks[2].trim_end_matches(';').parse().unwrap_or(l.rpersq);
                        } else if toks.len() >= 2 && toks[0] == "WIDTH" {
                            l.width_um = toks[1].trim_end_matches(';').parse().unwrap_or(l.width_um);
                        } else if toks.len() >= 3 && toks[0] == "DCCURRENTDENSITY" && toks[1] == "AVERAGE" {
                            // scalar form `DCCURRENTDENSITY AVERAGE <mA/um> ;`; a table form
                            // (next-line WIDTH/TABLEENTRIES) leaves toks[2] non-numeric -> skip.
                            l.dc_jmax = toks[2].trim_end_matches(';').parse().unwrap_or(l.dc_jmax);
                        } else if toks.len() >= 3 && toks[0] == "ACCURRENTDENSITY" && toks[1] == "RMS" {
                            l.ac_rms = toks[2].trim_end_matches(';').parse().unwrap_or(l.ac_rms);
                        } else if toks.len() >= 3 && toks[0] == "ACCURRENTDENSITY" && toks[1] == "PEAK" {
                            l.ac_peak = toks[2].trim_end_matches(';').parse().unwrap_or(l.ac_peak);
                        }
                    }
                }
            }
        }
        if layers.is_empty() {
            return Err(LefError("no LAYER blocks found".into()));
        }
        Ok(TechLef { layers })
    }

    pub fn load(path: &str) -> Result<TechLef, LefError> {
        let text = std::fs::read_to_string(path).map_err(|e| LefError(format!("{path}: {e}")))?;
        TechLef::parse(&text)
    }
}
