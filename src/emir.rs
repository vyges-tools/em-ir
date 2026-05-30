//! EM/IR analysis: assemble the PDN conductance system, solve, report.
//!
//! Builds the reduced free-node system from the PDN (pads are fixed-voltage
//! sources, loads inject current out of their node), solves for every node
//! voltage, then derives the worst **IR drop** (supply sag vs nominal) and the
//! per-segment current `|Δv|/R` checked against each layer's **EM** limit.
//!
//! Pure std — fully unit-tested offline.

use std::collections::HashMap;

use crate::pdn::PdnSpec;
use crate::solver::LinSys;

#[derive(Debug)]
pub enum EmIrError {
    Parse(String),
    Io(String),
    Solver(String),
    /// Reserved: electrothermal coupling (the BCD/power axis) isn't modeled in v0.
    ElectrothermalNotModeled,
}

impl std::fmt::Display for EmIrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmIrError::Parse(m) => write!(f, "parse error: {m}"),
            EmIrError::Io(m) => write!(f, "io error: {m}"),
            EmIrError::Solver(m) => write!(f, "solver error: {m}"),
            EmIrError::ElectrothermalNotModeled => write!(f, "electrothermal not modeled in v0"),
        }
    }
}
impl std::error::Error for EmIrError {}

#[derive(Debug, Clone)]
pub struct IrNode {
    pub node: String,
    pub voltage: f64,
    pub drop: f64,
    pub drop_pct: f64,
}

#[derive(Debug, Clone)]
pub struct EmViolation {
    pub a: String,
    pub b: String,
    pub layer: String,
    pub current: f64,
    pub limit: f64,
    pub ratio: f64,
}

#[derive(Debug, Clone)]
pub struct EmIrReport {
    pub vdd: f64,
    pub nodes: usize,
    pub worst_ir: Option<IrNode>,
    pub em_checked: usize,
    pub em_violations: Vec<EmViolation>,
    pub em_worst_ratio: f64,
}

pub fn analyze(spec: &PdnSpec) -> Result<EmIrReport, EmIrError> {
    let pad_v: HashMap<&str, f64> = spec.pads.iter().map(|(n, v)| (n.as_str(), *v)).collect();

    // free nodes = every node that is not a pad
    let mut free_idx: HashMap<String, usize> = HashMap::new();
    let mut free_names: Vec<String> = Vec::new();
    let see = |n: &str, free_idx: &mut HashMap<String, usize>, free_names: &mut Vec<String>| {
        if pad_v.contains_key(n) || free_idx.contains_key(n) {
            return;
        }
        free_idx.insert(n.to_string(), free_names.len());
        free_names.push(n.to_string());
    };
    for r in &spec.resistors {
        see(&r.a, &mut free_idx, &mut free_names);
        see(&r.b, &mut free_idx, &mut free_names);
    }
    for (n, _) in &spec.loads {
        see(n, &mut free_idx, &mut free_names);
    }

    let mut sys = LinSys::new(free_names.len());
    for r in &spec.resistors {
        let g = 1.0 / r.r;
        match (free_idx.get(&r.a), free_idx.get(&r.b)) {
            (Some(&i), Some(&j)) => {
                sys.diag[i] += g;
                sys.diag[j] += g;
                sys.offdiag[i].push((j, g));
                sys.offdiag[j].push((i, g));
            }
            (Some(&i), None) => {
                if let Some(&vp) = pad_v.get(r.b.as_str()) {
                    sys.diag[i] += g;
                    sys.rhs[i] += g * vp;
                }
            }
            (None, Some(&j)) => {
                if let Some(&vp) = pad_v.get(r.a.as_str()) {
                    sys.diag[j] += g;
                    sys.rhs[j] += g * vp;
                }
            }
            (None, None) => {} // pad-to-pad: no unknown
        }
    }
    for (node, amps) in &spec.loads {
        if let Some(&i) = free_idx.get(node) {
            sys.rhs[i] -= amps; // current drawn out of the node
        }
    }

    let x = sys.solve(20_000, 1e-10).map_err(|e| EmIrError::Solver(e.to_string()))?;

    let voltage = |n: &str| -> f64 {
        if let Some(&v) = pad_v.get(n) {
            v
        } else {
            x[free_idx[n]]
        }
    };

    // worst IR drop over the free nodes
    let mut worst_ir: Option<IrNode> = None;
    for (name, &i) in &free_idx {
        let v = x[i];
        let drop = spec.vdd - v;
        let cand = IrNode { node: name.clone(), voltage: v, drop, drop_pct: 100.0 * drop / spec.vdd };
        if worst_ir.as_ref().map(|w| cand.drop > w.drop).unwrap_or(true) {
            worst_ir = Some(cand);
        }
    }

    // EM check per segment that has a layer limit
    let mut em_violations = Vec::new();
    let mut em_checked = 0usize;
    let mut em_worst_ratio = 0.0f64;
    for r in &spec.resistors {
        let Some(layer) = &r.layer else { continue };
        let Some(&limit) = spec.em_limits.get(layer) else { continue };
        em_checked += 1;
        let current = (voltage(&r.a) - voltage(&r.b)).abs() / r.r;
        let ratio = current / limit;
        if ratio > em_worst_ratio {
            em_worst_ratio = ratio;
        }
        if ratio > 1.0 {
            em_violations.push(EmViolation {
                a: r.a.clone(),
                b: r.b.clone(),
                layer: layer.clone(),
                current,
                limit,
                ratio,
            });
        }
    }

    Ok(EmIrReport {
        vdd: spec.vdd,
        nodes: free_names.len(),
        worst_ir,
        em_checked,
        em_violations,
        em_worst_ratio,
    })
}
