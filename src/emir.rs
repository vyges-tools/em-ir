//! EM/IR analysis: assemble the PDN conductance system, solve, report.
//!
//! Builds the reduced free-node system from the PDN (pads are fixed-voltage
//! sources, loads inject current out of their node), solves for every node
//! voltage, then derives the worst **IR drop** (supply sag vs nominal) and the
//! per-segment current `|Δv|/R` checked against each layer's **EM** limit.
//!
//! Pure std — fully unit-tested offline.

use std::collections::HashMap;

use crate::pdn::{PdnSpec, Switch};
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

/// Worst dynamic (transient) voltage droop: the minimum supply voltage reached at
/// any node over the whole switching window, and when/where.
#[derive(Debug, Clone)]
pub struct DynIr {
    pub node: String,
    pub voltage: f64, // min supply voltage reached (V)
    pub drop: f64,    // vdd - voltage
    pub drop_pct: f64,
    pub time_ns: f64, // time of the worst droop
}

#[derive(Debug, Clone)]
pub struct EmIrReport {
    pub vdd: f64,
    pub nodes: usize,
    pub worst_ir: Option<IrNode>,
    pub em_checked: usize,
    pub em_violations: Vec<EmViolation>,
    pub em_worst_ratio: f64,
    /// Worst transient droop, when the PDN carries switching events (else None).
    pub dynamic: Option<DynIr>,
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
    for (n, _) in &spec.caps {
        see(n, &mut free_idx, &mut free_names);
    }
    for sw in &spec.switches {
        see(&sw.node, &mut free_idx, &mut free_names);
    }

    // Assemble the conductance network once into reusable base vectors: `base_diag`
    // + `offdiag` are the resistive conductances, `pad_rhs` the fixed pad injections.
    // The static and transient solves both build on these (transient adds C/dt).
    let n = free_names.len();
    let mut base_diag = vec![0.0; n];
    let mut offdiag: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    let mut pad_rhs = vec![0.0; n];
    for r in &spec.resistors {
        let g = 1.0 / r.r;
        match (free_idx.get(&r.a), free_idx.get(&r.b)) {
            (Some(&i), Some(&j)) => {
                base_diag[i] += g;
                base_diag[j] += g;
                offdiag[i].push((j, g));
                offdiag[j].push((i, g));
            }
            (Some(&i), None) => {
                if let Some(&vp) = pad_v.get(r.b.as_str()) {
                    base_diag[i] += g;
                    pad_rhs[i] += g * vp;
                }
            }
            (None, Some(&j)) => {
                if let Some(&vp) = pad_v.get(r.a.as_str()) {
                    base_diag[j] += g;
                    pad_rhs[j] += g * vp;
                }
            }
            (None, None) => {} // pad-to-pad: no unknown
        }
    }
    // per-free-node static current (out of node) and capacitance to ground (pF -> F)
    let mut dc = vec![0.0; n];
    for (node, amps) in &spec.loads {
        if let Some(&i) = free_idx.get(node) {
            dc[i] += amps;
        }
    }
    let mut cap = vec![0.0; n];
    for (node, c) in &spec.caps {
        if let Some(&i) = free_idx.get(node) {
            cap[i] += c * 1e-12;
        }
    }

    // static solve: base conductances, rhs = pad injections − static load current.
    let mut sys = LinSys::new(n);
    sys.diag.clone_from(&base_diag);
    sys.offdiag.clone_from(&offdiag);
    sys.rhs = pad_rhs.iter().zip(&dc).map(|(p, d)| p - d).collect();
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

    // Dynamic (transient) IR when the PDN carries switching events.
    let dynamic = if spec.is_dynamic() && n > 0 {
        Some(transient(spec, &free_idx, &free_names, &base_diag, &offdiag, &pad_rhs, &cap, &dc, &x)?)
    } else {
        None
    };

    Ok(EmIrReport {
        vdd: spec.vdd,
        nodes: free_names.len(),
        worst_ir,
        em_checked,
        em_violations,
        em_worst_ratio,
        dynamic,
    })
}

/// Current (A) drawn out of a switch's node at time `t_ns`: a triangular pulse of
/// total charge `energy/vdd` peaking at `t50` over `dur`.
fn switch_current(sw: &Switch, t_ns: f64, vdd: f64) -> f64 {
    let half = sw.dur_ns / 2.0;
    let d = t_ns - sw.t50_ns;
    if d.abs() >= half {
        return 0.0;
    }
    let q = (sw.energy_pj * 1e-12) / vdd; // Coulombs
    let ipk = 2.0 * q / (sw.dur_ns * 1e-9); // triangle area = q
    ipk * (1.0 - d.abs() / half)
}

/// Backward-Euler transient solve over the switching window. Each timestep is a
/// conductance solve with `C/dt` added to the diagonal and `i(t)` + `(C/dt)·v_prev`
/// in the rhs; tracks the deepest droop reached at any node. The capacitance smooths
/// the response, but the instantaneous current peaks make the dynamic droop worse
/// than the static IR — which is the point of the analysis.
#[allow(clippy::too_many_arguments)]
fn transient(
    spec: &PdnSpec,
    free_idx: &HashMap<String, usize>,
    free_names: &[String],
    base_diag: &[f64],
    offdiag: &[Vec<(usize, f64)>],
    pad_rhs: &[f64],
    cap: &[f64],
    dc: &[f64],
    x0: &[f64],
) -> Result<DynIr, EmIrError> {
    let n = free_names.len();
    let last = spec.switches.iter().map(|s| s.t50_ns + s.dur_ns).fold(0.0, f64::max);
    let min_dur = spec.switches.iter().map(|s| s.dur_ns).fold(f64::INFINITY, f64::min);
    let dt_ns = (min_dur / 10.0).max(1e-3); // resolve the pulse; >= 1 ps
    let tstop_ns = last + 1.0; // a little settle past the last event
    let dt = dt_ns * 1e-9;

    let mut v_prev = x0.to_vec();
    let mut vmin = x0.to_vec();
    let mut tmin = vec![0.0; n];
    let mut t_ns = 0.0;
    while t_ns < tstop_ns {
        t_ns += dt_ns;
        let mut sys = LinSys::new(n);
        sys.offdiag = offdiag.to_vec();
        for k in 0..n {
            sys.diag[k] = base_diag[k] + cap[k] / dt;
            sys.rhs[k] = pad_rhs[k] - dc[k] + (cap[k] / dt) * v_prev[k];
        }
        for sw in &spec.switches {
            if let Some(&k) = free_idx.get(&sw.node) {
                sys.rhs[k] -= switch_current(sw, t_ns, spec.vdd);
            }
        }
        let v = sys.solve(20_000, 1e-10).map_err(|e| EmIrError::Solver(e.to_string()))?;
        for k in 0..n {
            if v[k] < vmin[k] {
                vmin[k] = v[k];
                tmin[k] = t_ns;
            }
        }
        v_prev = v;
    }

    let worst = (0..n).min_by(|&a, &b| vmin[a].total_cmp(&vmin[b])).unwrap_or(0);
    let v = vmin[worst];
    let drop = spec.vdd - v;
    Ok(DynIr {
        node: free_names[worst].clone(),
        voltage: v,
        drop,
        drop_pct: 100.0 * drop / spec.vdd,
        time_ns: tmin[worst],
    })
}
