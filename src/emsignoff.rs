//! Extracted-SPEF electromigration sign-off — screen *signal*-net metal, not just
//! the power grid. The existing DEF+LEF path screens the PDN; this path takes the
//! per-segment geometry the RCX front end emits alongside SPEF (the loom
//! [`EmGeom`](vyges_loom::emgeom::EmGeom) sidecar: layer, width, length per metal
//! segment) plus a per-net current, and checks each segment's current density
//! against the tech-LEF `DCCURRENTDENSITY` (and AC RMS/PEAK when supplied).
//!
//! Limit: `I_max = J_layer[mA/µm] · width[µm] · 1e-3` (A) — the same width-scaled
//! LEF form the PDN path uses. A signal net's series metal carries the net current,
//! so each of its segments is screened at that current. Results reuse the shared
//! [`EmViolation`]/[`EmIrReport`] so ranking, `EM-VIOL` events, and the exit-3 gate
//! are identical to PDN EM.

use std::collections::BTreeMap;

use vyges_loom::emgeom::EmGeom;
use vyges_loom::lef::Lef;

use crate::emir::{EmIrReport, EmViolation};

/// Per-net current in **mA**: average (DC), and optional RMS / peak (AC).
#[derive(Debug, Clone, Default)]
pub struct CurrentMap {
    pub avg: BTreeMap<String, f64>,
    pub rms: BTreeMap<String, f64>,
    pub peak: BTreeMap<String, f64>,
    /// Fallback average current (mA) for nets absent from the map (0 = skip).
    pub default_avg_ma: f64,
}

impl CurrentMap {
    /// Parse a current map: `<net> <avg_mA> [rms_mA] [peak_mA]`, `#` comments.
    /// Robust — malformed lines are skipped (never crash on a bad row).
    pub fn parse(text: &str) -> CurrentMap {
        let mut m = CurrentMap::default();
        for raw in text.lines() {
            let t = raw.split('#').next().unwrap_or("").trim();
            if t.is_empty() {
                continue;
            }
            let k: Vec<&str> = t.split_whitespace().collect();
            if k.len() < 2 {
                continue;
            }
            let net = k[0].to_string();
            if let Ok(a) = k[1].parse::<f64>() {
                m.avg.insert(net.clone(), a);
            } else {
                continue;
            }
            if let Some(r) = k.get(2).and_then(|s| s.parse::<f64>().ok()) {
                m.rms.insert(net.clone(), r);
            }
            if let Some(p) = k.get(3).and_then(|s| s.parse::<f64>().ok()) {
                m.peak.insert(net.clone(), p);
            }
        }
        m
    }

    pub fn load(path: &str) -> std::io::Result<CurrentMap> {
        Ok(CurrentMap::parse(&std::fs::read_to_string(path)?))
    }
}

/// Screen every geometry segment's current density against the tech-LEF limits.
/// `em_checked` counts segments that had a DC limit; a violation is `I > I_max`.
pub fn screen(geom: &EmGeom, lef: &Lef, cur: &CurrentMap) -> EmIrReport {
    let mut viols: Vec<EmViolation> = Vec::new();
    let mut checked = 0usize;
    let mut worst = 0.0f64;

    // one screen against a (limit-density mA/µm, current mA) pair
    let check = |kind: &str,
                 seg: &vyges_loom::emgeom::SegGeom,
                 jmax_ma_um: f64,
                 cur_ma: Option<f64>,
                 viols: &mut Vec<EmViolation>,
                 worst: &mut f64| {
        if jmax_ma_um <= 0.0 {
            return false; // no limit on this layer for this kind
        }
        let cur_ma = match cur_ma {
            Some(c) if c > 0.0 => c,
            _ => return true, // limit exists but no current supplied → checked, no viol
        };
        let limit_a = jmax_ma_um * seg.width_um * 1e-3;
        let current_a = cur_ma * 1e-3;
        let ratio = if limit_a > 0.0 {
            current_a / limit_a
        } else {
            0.0
        };
        if ratio > *worst {
            *worst = ratio;
        }
        if ratio > 1.0 {
            viols.push(EmViolation {
                kind: kind.to_string(),
                a: seg.a.clone(),
                b: seg.b.clone(),
                layer: seg.layer.clone(),
                current: current_a,
                limit: limit_a,
                ratio,
            });
        }
        true
    };

    for seg in &geom.segs {
        let layer = match lef.layers.get(&seg.layer) {
            Some(l) => l,
            None => continue, // layer not in LEF → cannot screen
        };
        let net_avg = cur
            .avg
            .get(&seg.net)
            .copied()
            .or({ (cur.default_avg_ma > 0.0).then_some(cur.default_avg_ma) });
        let dc_checked = check("dc", seg, layer.dc_jmax, net_avg, &mut viols, &mut worst);
        // AC screens only when the net has an RMS/peak current supplied
        check(
            "rms",
            seg,
            layer.ac_rms,
            cur.rms.get(&seg.net).copied(),
            &mut viols,
            &mut worst,
        );
        check(
            "peak",
            seg,
            layer.ac_peak,
            cur.peak.get(&seg.net).copied(),
            &mut viols,
            &mut worst,
        );
        if dc_checked {
            checked += 1;
        }
    }

    EmIrReport {
        vdd: 0.0,
        nodes: geom.segs.len(),
        worst_ir: None,
        em_checked: checked,
        em_violations: viols,
        em_worst_ratio: worst,
        dynamic: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vyges_loom::emgeom::SegGeom;

    fn lef() -> Lef {
        // met1 DC limit 1.5 mA/µm; met2 2.0
        Lef::parse(
            "LAYER met1\n  TYPE ROUTING ;\n  WIDTH 0.14 ;\n  DCCURRENTDENSITY AVERAGE 1.5 ;\nEND met1\n\
             LAYER met2\n  TYPE ROUTING ;\n  WIDTH 0.20 ;\n  DCCURRENTDENSITY AVERAGE 2.0 ;\nEND met2\n",
        )
        .unwrap()
    }

    fn geom() -> EmGeom {
        EmGeom {
            design: "t".into(),
            segs: vec![
                // met1, 0.14 µm wide → limit = 1.5*0.14 = 0.21 mA
                SegGeom {
                    net: "clk".into(),
                    a: "clk".into(),
                    b: "clk^met1".into(),
                    layer: "met1".into(),
                    width_um: 0.14,
                    length_um: 10.0,
                    res_ohm: 9.0,
                },
                // met2, 0.20 µm wide → limit = 2.0*0.20 = 0.40 mA
                SegGeom {
                    net: "clk".into(),
                    a: "clk".into(),
                    b: "clk^met2".into(),
                    layer: "met2".into(),
                    width_um: 0.20,
                    length_um: 4.0,
                    res_ohm: 2.5,
                },
            ],
        }
    }

    #[test]
    fn flags_overcurrent_segment() {
        let mut cur = CurrentMap::default();
        cur.avg.insert("clk".into(), 0.30); // 0.30 mA
        let rep = screen(&geom(), &lef(), &cur);
        assert_eq!(rep.em_checked, 2);
        // met1 limit 0.21 mA < 0.30 → violation; met2 limit 0.40 > 0.30 → ok
        assert_eq!(rep.em_violations.len(), 1);
        let v = &rep.em_violations[0];
        assert_eq!(v.layer, "met1");
        assert!(v.ratio > 1.0);
        assert!((rep.em_worst_ratio - (0.30 / 0.21)).abs() < 1e-6);
    }

    #[test]
    fn clean_when_under_limit() {
        let mut cur = CurrentMap::default();
        cur.avg.insert("clk".into(), 0.10);
        let rep = screen(&geom(), &lef(), &cur);
        assert!(rep.em_violations.is_empty());
        assert_eq!(rep.em_checked, 2);
    }

    #[test]
    fn no_current_is_checked_but_not_violated() {
        let rep = screen(&geom(), &lef(), &CurrentMap::default());
        assert_eq!(rep.em_checked, 2); // limits exist
        assert!(rep.em_violations.is_empty()); // but no current supplied
    }
}
