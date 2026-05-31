//! PDN extraction: DEF power-grid geometry + tech-LEF sheet resistances -> the
//! resistor network (`PdnSpec`) the solver consumes.
//!
//! Each special-net wire segment becomes a resistor `R = rpersq · L/W` (squares of
//! sheet resistance), with a node at every polyline point keyed by `(layer, x, y)`.
//! Co-located points on different layers are bridged by a via resistor wherever the
//! DEF places a via. The `pad_layer` nodes are tied to the supply (the top-metal /
//! C4 supply plane); the static load current is spread uniformly over the remaining
//! nodes (per-instance loads from DEF COMPONENTS is the follow-up).

use std::collections::BTreeMap;

use crate::def::Def;
use crate::job::EmIrJob;
use crate::lef::TechLef;
use crate::pdn::{PdnSpec, Resistor};

fn node(layer: &str, x: i64, y: i64) -> String {
    format!("{layer}_{x}_{y}")
}

/// Build a `PdnSpec` from the extracted DEF power net + LEF resistances + job params.
pub fn extract(def: &Def, lef: &TechLef, job: &EmIrJob) -> Result<PdnSpec, String> {
    let net = def.power_net().ok_or_else(|| "no power net in DEF".to_string())?;
    if net.segs.is_empty() {
        return Err(format!("power net {:?} has no routed segments", net.name));
    }
    let dbu = def.dbu;

    // wire resistors: R = rpersq · (length / width), lengths/widths in microns.
    let mut resistors: Vec<Resistor> = Vec::new();
    // layers present at each point — used to bridge vias between metals.
    let mut at_point: BTreeMap<(i64, i64), Vec<String>> = BTreeMap::new();
    let note_pt = |x: i64, y: i64, layer: &str, m: &mut BTreeMap<(i64, i64), Vec<String>>| {
        let e = m.entry((x, y)).or_default();
        if !e.iter().any(|l| l == layer) {
            e.push(layer.to_string());
        }
    };
    for s in &net.segs {
        let lr = lef.layers.get(&s.layer);
        let rpersq = lr.map(|l| l.rpersq).unwrap_or(0.0);
        if rpersq <= 0.0 {
            return Err(format!("layer {:?} has no RESISTANCE RPERSQ in the LEF", s.layer));
        }
        // width: the segment's own width if given, else the LEF default.
        let w_um = if s.width_dbu > 0.0 {
            s.width_dbu / dbu
        } else {
            lr.map(|l| l.width_um).unwrap_or(0.0)
        };
        if w_um <= 0.0 {
            return Err(format!("segment on {:?} has no width (DEF or LEF)", s.layer));
        }
        let len_um = (((s.x2 - s.x1) as f64).hypot((s.y2 - s.y1) as f64)) / dbu;
        let r = rpersq * len_um / w_um;
        if r > 0.0 {
            resistors.push(Resistor {
                a: node(&s.layer, s.x1, s.y1),
                b: node(&s.layer, s.x2, s.y2),
                r,
                layer: Some(s.layer.clone()),
            });
        }
        note_pt(s.x1, s.y1, &s.layer, &mut at_point);
        note_pt(s.x2, s.y2, &s.layer, &mut at_point);
    }

    // via resistors: at each via point bridge every pair of distinct layers present.
    for &(x, y) in &net.vias {
        if let Some(layers) = at_point.get(&(x, y)) {
            for i in 0..layers.len() {
                for j in (i + 1)..layers.len() {
                    resistors.push(Resistor {
                        a: node(&layers[i], x, y),
                        b: node(&layers[j], x, y),
                        r: job.via_res,
                        layer: Some("via".to_string()),
                    });
                }
            }
        }
    }

    // pads: every node on the supply (pad) layer is held at vdd.
    let mut pads: Vec<(String, f64)> = Vec::new();
    let mut pad_nodes: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for ((x, y), layers) in &at_point {
        if layers.iter().any(|l| l == &job.pad_layer) {
            let n = node(&job.pad_layer, *x, *y);
            if pad_nodes.insert(n.clone()) {
                pads.push((n, job.vdd));
            }
        }
    }
    if pads.is_empty() {
        return Err(format!("pad_layer {:?} has no nodes in the DEF power grid", job.pad_layer));
    }

    // loads: spread the total static current uniformly over the non-pad nodes.
    let mut load_nodes: Vec<String> = Vec::new();
    for ((x, y), layers) in &at_point {
        for l in layers {
            if l != &job.pad_layer {
                load_nodes.push(node(l, *x, *y));
            }
        }
    }
    load_nodes.sort();
    load_nodes.dedup();
    let mut loads: Vec<(String, f64)> = Vec::new();
    if job.total_current > 0.0 && !load_nodes.is_empty() {
        let per = job.total_current / load_nodes.len() as f64;
        loads = load_nodes.into_iter().map(|n| (n, per)).collect();
    }

    Ok(PdnSpec { vdd: job.vdd, pads, resistors, loads, ..Default::default() })
}
