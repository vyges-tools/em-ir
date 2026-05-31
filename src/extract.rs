//! PDN extraction: DEF power-grid geometry + tech-LEF sheet resistances -> the
//! resistor network (`PdnSpec`) the solver consumes.
//!
//! Each special-net wire segment becomes a resistor `R = rpersq · L/W` (squares of
//! sheet resistance), with a node at every polyline point keyed by `(layer, x, y)`.
//!
//! **Via-stack resolution.** Vias often land in the *middle* of a crossing stripe
//! and stacks are written as single-point via-only statements (`NEW met3 0 ( x y )
//! viaN`). So before building resistors we **split every wire segment at any via
//! point lying on it** (inserting a node there), and we keep the single-point
//! landings as nodes too. At each via location we then connect the **adjacent metal
//! layers** present (sorted by metal index — a real via stack is met1-via-met2-…),
//! not just segment endpoints — so a met1-rail → via-stack → met5-strap path is
//! electrically continuous.
//!
//! The `pad_layer` nodes are tied to the supply (top-metal / C4 plane); the static
//! load current is spread uniformly over the remaining nodes (per-instance loads
//! from DEF COMPONENTS is the follow-up).

use std::collections::{BTreeMap, BTreeSet};

use crate::def::Def;
use crate::job::EmIrJob;
use crate::lef::TechLef;
use crate::pdn::{PdnSpec, Resistor, Switch};

/// Parse a char-derived `cell <energy_pJ> [leakage_nW]` power map.
fn parse_power_map(text: &str) -> BTreeMap<String, f64> {
    let mut m = BTreeMap::new();
    for line in text.lines() {
        let l = line.split('#').next().unwrap_or("").trim();
        if l.is_empty() {
            continue;
        }
        let mut it = l.split_whitespace();
        if let (Some(cell), Some(e)) = (it.next(), it.next()) {
            if let Ok(v) = e.parse::<f64>() {
                m.insert(cell.to_string(), v);
            }
        }
    }
    m
}

fn node(layer: &str, x: i64, y: i64) -> String {
    format!("{layer}_{x}_{y}")
}

/// Metal-stack ordering for a layer name: `li` = 0, `metN` = N, else large (so
/// unknown layers sort last and don't get spuriously bridged into a stack).
fn metal_index(layer: &str) -> i32 {
    if layer.eq_ignore_ascii_case("li") || layer.eq_ignore_ascii_case("li1") {
        return 0;
    }
    let digits: String = layer.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse::<i32>().unwrap_or(9999)
}

/// Is point `(px,py)` strictly on the open segment `p1->p2` (collinear & between)?
fn on_segment(x1: i64, y1: i64, x2: i64, y2: i64, px: i64, py: i64) -> bool {
    let (dx, dy) = (x2 - x1, y2 - y1);
    if (x2 - x1) * (py - y1) - (y2 - y1) * (px - x1) != 0 {
        return false; // not collinear
    }
    let dot = (px - x1) * dx + (py - y1) * dy;
    let len2 = dx * dx + dy * dy;
    dot > 0 && dot < len2 // strictly between the endpoints
}

/// Build a `PdnSpec` from the extracted DEF power net + LEF resistances + job params.
pub fn extract(def: &Def, lef: &TechLef, job: &EmIrJob) -> Result<PdnSpec, String> {
    let net = def.power_net().ok_or_else(|| "no power net in DEF".to_string())?;
    if net.segs.is_empty() {
        return Err(format!("power net {:?} has no routed segments", net.name));
    }
    let dbu = def.dbu;
    let via_locs: BTreeSet<(i64, i64)> = net.vias.iter().copied().collect();

    // layers present at each point -> via bridging; populated from every node we make.
    let mut at_point: BTreeMap<(i64, i64), BTreeSet<String>> = BTreeMap::new();
    let note = |x: i64, y: i64, layer: &str, m: &mut BTreeMap<(i64, i64), BTreeSet<String>>| {
        m.entry((x, y)).or_default().insert(layer.to_string());
    };

    // wire resistors: split each segment at the via points lying on it, so a via that
    // lands mid-stripe gets a node it can bridge through.
    let mut resistors: Vec<Resistor> = Vec::new();
    for s in &net.segs {
        let lr = lef.layers.get(&s.layer);
        let rpersq = lr.map(|l| l.rpersq).unwrap_or(0.0);
        if rpersq <= 0.0 {
            return Err(format!("layer {:?} has no RESISTANCE RPERSQ in the LEF", s.layer));
        }
        let w_um = if s.width_dbu > 0.0 { s.width_dbu / dbu } else { lr.map(|l| l.width_um).unwrap_or(0.0) };
        if w_um <= 0.0 {
            return Err(format!("segment on {:?} has no width (DEF or LEF)", s.layer));
        }
        // collect split points (via locations on this segment), ordered along it.
        let mut cuts: Vec<(i64, i64)> = via_locs
            .iter()
            .copied()
            .filter(|&(px, py)| on_segment(s.x1, s.y1, s.x2, s.y2, px, py))
            .collect();
        let (dx, dy) = (s.x2 - s.x1, s.y2 - s.y1);
        cuts.sort_by_key(|&(px, py)| (px - s.x1) * dx + (py - s.y1) * dy);
        // emit the chain p1 -> cut1 -> ... -> p2
        let mut chain = vec![(s.x1, s.y1)];
        chain.extend(cuts);
        chain.push((s.x2, s.y2));
        for w in chain.windows(2) {
            let (ax, ay) = w[0];
            let (bx, by) = w[1];
            note(ax, ay, &s.layer, &mut at_point);
            note(bx, by, &s.layer, &mut at_point);
            if ax == bx && ay == by {
                continue;
            }
            let len_um = (((bx - ax) as f64).hypot((by - ay) as f64)) / dbu;
            resistors.push(Resistor {
                a: node(&s.layer, ax, ay),
                b: node(&s.layer, bx, by),
                r: rpersq * len_um / w_um,
                layer: Some(s.layer.clone()),
            });
        }
    }
    // single-point via landings (and all listed points) are nodes too.
    for (layer, x, y) in &net.points {
        note(*x, *y, layer, &mut at_point);
    }

    // via resistors: at each via location connect the adjacent metal layers present.
    for &(x, y) in &via_locs {
        let Some(layers) = at_point.get(&(x, y)) else { continue };
        let mut ls: Vec<&String> = layers.iter().collect();
        ls.sort_by_key(|l| metal_index(l));
        for w in ls.windows(2) {
            resistors.push(Resistor {
                a: node(w[0], x, y),
                b: node(w[1], x, y),
                r: job.via_res,
                layer: Some("via".to_string()),
            });
        }
    }

    // pads: every node on the supply (pad) layer is held at vdd.
    let mut pads: Vec<(String, f64)> = Vec::new();
    let mut seen_pad: BTreeSet<String> = BTreeSet::new();
    for ((x, y), layers) in &at_point {
        if layers.contains(&job.pad_layer) {
            let n = node(&job.pad_layer, *x, *y);
            if seen_pad.insert(n.clone()) {
                pads.push((n, job.vdd));
            }
        }
    }
    if pads.is_empty() {
        return Err(format!("pad_layer {:?} has no nodes in the DEF power grid", job.pad_layer));
    }

    // load nodes = the lowest non-pad metal layer (where cells tap the supply rails).
    let lowest = at_point
        .values()
        .flatten()
        .filter(|l| *l != &job.pad_layer)
        .map(|l| metal_index(l))
        .min();
    let mut lnodes: Vec<(String, i64, i64)> = Vec::new();
    for ((x, y), layers) in &at_point {
        for l in layers {
            if l != &job.pad_layer && Some(metal_index(l)) == lowest {
                lnodes.push((node(l, *x, *y), *x, *y));
            }
        }
    }

    let mut loads: Vec<(String, f64)> = Vec::new();
    let mut switches: Vec<Switch> = Vec::new();
    let mut caps: Vec<(String, f64)> = Vec::new();

    if !job.power_map.is_empty() && !def.comps.is_empty() && !lnodes.is_empty() {
        // The seam on silicon: each instance's current = its cell's char switching
        // energy, landed on the nearest rail node; the same energy drives a switch
        // event for dynamic IR. Static avg current = (energy/vdd) · f · activity.
        let text = std::fs::read_to_string(job.resolve(&job.power_map)).map_err(|e| e.to_string())?;
        let pmap = parse_power_map(&text);
        let f = job.clock_ghz * 1e9;
        let mut sload: BTreeMap<String, f64> = BTreeMap::new();
        let mut senergy: BTreeMap<String, f64> = BTreeMap::new();
        for c in &def.comps {
            let e = match pmap.get(&c.cell) {
                Some(&e) if e > 0.0 => e,
                _ => continue, // fillers/decaps/taps and uncharacterized cells: no current
            };
            let nn = lnodes
                .iter()
                .min_by_key(|(_, x, y)| (x - c.x) * (x - c.x) + (y - c.y) * (y - c.y));
            if let Some((name, _, _)) = nn {
                let q = e * 1e-12 / job.vdd; // Coulombs per switch
                *sload.entry(name.clone()).or_default() += q * f * job.activity;
                *senergy.entry(name.clone()).or_default() += e;
            }
        }
        loads = sload.into_iter().collect();
        switches = senergy
            .into_iter()
            .map(|(node, energy_pj)| Switch {
                node,
                energy_pj,
                t50_ns: job.switch_t_ns,
                dur_ns: job.switch_dur_ns,
            })
            .collect();
        if job.node_cap_pf > 0.0 {
            caps = lnodes.iter().map(|(n, _, _)| (n.clone(), job.node_cap_pf)).collect();
        }
    } else if job.total_current > 0.0 && !lnodes.is_empty() {
        // uniform fallback: spread the total current over the rail nodes.
        let per = job.total_current / lnodes.len() as f64;
        loads = lnodes.iter().map(|(n, _, _)| (n.clone(), per)).collect();
    }

    Ok(PdnSpec { vdd: job.vdd, pads, resistors, loads, switches, caps, ..Default::default() })
}
