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

/// Read a `cell <value> …` map file (resolved against the job dir); empty path -> {}.
fn read_map(job: &EmIrJob, rel: &str) -> Result<BTreeMap<String, f64>, String> {
    if rel.is_empty() {
        return Ok(BTreeMap::new());
    }
    let text = std::fs::read_to_string(job.resolve(rel)).map_err(|e| e.to_string())?;
    Ok(parse_power_map(&text))
}

/// Parse a `cell <value> [extra…]` map (char switching energy pJ, or decap pF).
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
    let net = def
        .power_net()
        .ok_or_else(|| "no power net in DEF".to_string())?;
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

    // ── where each instance's current enters the grid ────────────────────────────────
    //
    // A cell taps the supply from the rail it sits on, at its own position along that
    // rail. Landing its current on the nearest EXISTING node instead means the current
    // never crosses the rail resistance between the cell and that node — and since a
    // rail is one long segment until something splits it, "the nearest node" is usually
    // an endpoint far away.
    //
    // Measured against PDNSim on a routed sky130 block, that was worth 3.2x: 15 704
    // current-carrying instances collapsed onto 834 injection nodes, our met1 rails were
    // 750 resistors averaging 29.9 ohm where PDNSim had 32 603 averaging 0.685 ohm for
    // the SAME total resistance, and we under-reported worst IR drop by that factor. The
    // resistance was all present; the current was entering in the wrong places.
    //
    // So each instance gets a tap point on its rail, and the segment is split there — the
    // same mechanism vias already use. PDNSim reaches the same end differently, hanging an
    // ITermNode off the rail per terminal through a 1 mohm stub.
    let pmap = read_map(job, &job.power_map)?;
    let dmap = read_map(job, &job.decap_map)?;
    let imap = read_map(job, &job.current_map)?;

    // The rail layer: the lowest metal the power net is drawn on that is not the supply
    // plane. This is where standard cells connect.
    let rail_layer: Option<String> = net
        .segs
        .iter()
        .map(|s| &s.layer)
        .filter(|l| **l != job.pad_layer)
        .min_by_key(|l| metal_index(l))
        .cloned();

    // instance name -> the point on the rail where its current enters
    let mut taps: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let mut tap_points: BTreeSet<(i64, i64)> = BTreeSet::new();
    if let Some(rl) = &rail_layer {
        let rails: Vec<&crate::def::Seg> =
            net.segs.iter().filter(|s| s.layer == *rl).collect();
        for c in &def.comps {
            // only instances that will actually carry current or capacitance
            if !imap.contains_key(&c.name)
                && !pmap.contains_key(&c.cell)
                && !dmap.contains_key(&c.cell)
            {
                continue;
            }
            let mut best: Option<(i64, (i64, i64))> = None;
            for s in &rails {
                // Project onto the segment. Rails are axis-aligned, and only an
                // axis-aligned projection is guaranteed to land EXACTLY on the segment —
                // a rounded diagonal projection would name a node that does not exist and
                // the current would vanish silently.
                let p = if s.y1 == s.y2 {
                    (c.x.clamp(s.x1.min(s.x2), s.x1.max(s.x2)), s.y1)
                } else if s.x1 == s.x2 {
                    (s.x1, c.y.clamp(s.y1.min(s.y2), s.y1.max(s.y2)))
                } else {
                    continue;
                };
                let d = (p.0 - c.x) * (p.0 - c.x) + (p.1 - c.y) * (p.1 - c.y);
                if best.map(|(bd, _)| d < bd).unwrap_or(true) {
                    best = Some((d, p));
                }
            }
            if let Some((_, p)) = best {
                taps.insert(c.name.clone(), p);
                tap_points.insert(p);
            }
        }
    }

    // wire resistors: split each segment at the via points lying on it, so a via that
    // lands mid-stripe gets a node it can bridge through, and at the tap points where
    // instance current enters.
    let mut resistors: Vec<Resistor> = Vec::new();
    for s in &net.segs {
        let lr = lef.layers.get(&s.layer);
        let rpersq = lr.map(|l| l.rpersq).unwrap_or(0.0);
        if rpersq <= 0.0 {
            return Err(format!(
                "layer {:?} has no RESISTANCE RPERSQ in the LEF",
                s.layer
            ));
        }
        let w_um = if s.width_dbu > 0.0 {
            s.width_dbu / dbu
        } else {
            lr.map(|l| l.width_um).unwrap_or(0.0)
        };
        if w_um <= 0.0 {
            return Err(format!(
                "segment on {:?} has no width (DEF or LEF)",
                s.layer
            ));
        }
        // EM limits (A) for this wire = LEF current-density (mA/um) × its width (um).
        let lim = |j: f64| if j > 0.0 { Some(j * w_um * 1e-3) } else { None };
        let em_limit = lim(lr.map(|l| l.dc_jmax).unwrap_or(0.0));
        let em_rms_limit = lim(lr.map(|l| l.ac_rms).unwrap_or(0.0));
        let em_peak_limit = lim(lr.map(|l| l.ac_peak).unwrap_or(0.0));
        // collect split points (via locations and instance taps on this segment), ordered
        // along it. Deduplicated: many cells on one rail can project to the same point, and
        // a tap can coincide with a via.
        let mut cutset: BTreeSet<(i64, i64)> = via_locs
            .iter()
            .copied()
            .filter(|&(px, py)| on_segment(s.x1, s.y1, s.x2, s.y2, px, py))
            .collect();
        cutset.extend(
            tap_points
                .iter()
                .copied()
                .filter(|&(px, py)| on_segment(s.x1, s.y1, s.x2, s.y2, px, py)),
        );
        let mut cuts: Vec<(i64, i64)> = cutset.into_iter().collect();
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
                em_limit,
                em_rms_limit,
                em_peak_limit,
            });
        }
    }
    // single-point via landings (and all listed points) are nodes too.
    for (layer, x, y) in &net.points {
        note(*x, *y, layer, &mut at_point);
    }

    // Per-location via resistance, from the via definition the DEF names there.
    //
    // A via's resistance is its cut layer's LEF `RESISTANCE` (stated **per cut**) divided by
    // the number of cuts in the array. Both come from the DEF `VIAS` entry: `LAYERS <below>
    // <cut> <above>` names the cut layer, `ROWCOL` gives the count. Neither is recoverable
    // from the via's coordinate, and the via's *name* is not usable either — sky130 PDN vias
    // are called `via2_3_…` while bridging met1->met2.
    //
    // Measured against PDNSim on a routed sky130 block: a single flat resistance was 5.6x to
    // 13.2x too high across the four via classes present, because those arrays carry 5, 4, 4
    // and 1 cuts against per-cut values spanning 0.38 to 4.50 ohm. `via_res` remains the
    // fallback for a via whose definition or cut-layer resistance the files do not state.
    //
    // Keyed on the LAYER PAIR as well as the point, because a PDN via stack puts several via
    // definitions at the SAME (x,y) — met1->met2, met2->met3, met3->met4 all land on one
    // coordinate. Keyed on the point alone one definition wins and prices the whole stack:
    // that cost 3.6 % on the first measurement here, and it is small only because those three
    // cut layers happen to be similar. A stack spanning mcon (9.30 ohm) to via4 (0.38) would
    // be wrong by an order of magnitude.
    let mut via_r_at: BTreeMap<(i64, i64, String, String), f64> = BTreeMap::new();
    if let Some(n) = def.power_net() {
        for (name, x, y) in &n.via_names {
            let Some(vd) = def.via_defs.get(name) else {
                continue;
            };
            let Some(cut_res) = lef.layers.get(&vd.cut_layer).map(|l| l.cut_res) else {
                continue;
            };
            if cut_res <= 0.0 || vd.cuts == 0 {
                continue;
            }
            // order the pair the way the stack walk below sees it (low metal first)
            let (lo, hi) = if metal_index(&vd.below) <= metal_index(&vd.above) {
                (vd.below.clone(), vd.above.clone())
            } else {
                (vd.above.clone(), vd.below.clone())
            };
            via_r_at.insert((*x, *y, lo, hi), cut_res / vd.cuts as f64);
        }
    }

    // via resistors: at each via location connect the adjacent metal layers present.
    for &(x, y) in &via_locs {
        let Some(layers) = at_point.get(&(x, y)) else {
            continue;
        };
        let mut ls: Vec<&String> = layers.iter().collect();
        ls.sort_by_key(|l| metal_index(l));
        for w in ls.windows(2) {
            let r = via_r_at
                .get(&(x, y, w[0].clone(), w[1].clone()))
                .copied()
                .unwrap_or(job.via_res);
            resistors.push(Resistor {
                a: node(w[0], x, y),
                b: node(w[1], x, y),
                r,
                layer: Some("via".to_string()),
                em_limit: None, // via EM (per-cut) is a follow-up
                em_rms_limit: None,
                em_peak_limit: None,
            });
        }
    }

    // Supply sources: the grid nodes the design's own power PIN covers.
    //
    // A DEF `PINS` entry states where supply actually enters the die — port rectangles on
    // one or more layers, placed and oriented. Every grid node inside one of those shapes is
    // held at vdd; everything else has to be reached THROUGH the grid.
    //
    // The fallback, when a design declares no power pin, is every node on `pad_layer`. That
    // is deliberately the same precedence PDNSim uses (`generateSourceNodes`: an explicit
    // source file, else the net's bterms, else all top-layer nodes), and the fallback is the
    // weaker model: holding a whole layer ideal deletes resistance the supply really crosses,
    // so it UNDER-reports IR drop. On the sky130 block used for the PDNSim correlation the
    // design declares 9 port rectangles across met4 and met5 where the fallback would have
    // held all 28 met5 nodes — so this is not a corner case.
    let mut pads: Vec<(String, f64)> = Vec::new();
    let mut seen_pad: BTreeSet<String> = BTreeSet::new();

    let net_name = net.name.as_str();
    let pin_shapes: Vec<&(String, i64, i64, i64, i64)> = def
        .pins
        .iter()
        .filter(|p| p.use_power || p.use_ground)
        .filter(|p| p.net == net_name || p.name == net_name)
        .flat_map(|p| p.shapes.iter())
        .collect();

    if !pin_shapes.is_empty() {
        for ((x, y), layers) in &at_point {
            for layer in layers {
                let covered = pin_shapes.iter().any(|(pl, x1, y1, x2, y2)| {
                    pl == layer && *x >= *x1 && *x <= *x2 && *y >= *y1 && *y <= *y2
                });
                if covered {
                    let n = node(layer, *x, *y);
                    if seen_pad.insert(n.clone()) {
                        pads.push((n, job.vdd));
                    }
                }
            }
        }
    }

    if pads.is_empty() {
        for ((x, y), layers) in &at_point {
            if layers.contains(&job.pad_layer) {
                let n = node(&job.pad_layer, *x, *y);
                if seen_pad.insert(n.clone()) {
                    pads.push((n, job.vdd));
                }
            }
        }
    }
    if pads.is_empty() {
        return Err(format!(
            "net {net_name:?} has no power pin shapes, and pad_layer {:?} has no nodes in the \
             DEF power grid",
            job.pad_layer
        ));
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

    // Where an instance's current enters: the tap node split into its own rail above.
    // Falls back to the nearest existing rail node only when no rail segment could be
    // projected onto — which under-reports drop (see the tap comment), so it is counted
    // rather than left to be inferred from a quietly smaller answer.
    let mut untapped = 0usize;
    let rail = rail_layer.clone().unwrap_or_default();
    let nearest = |cx: i64, cy: i64| -> Option<String> {
        lnodes
            .iter()
            .min_by_key(|(_, x, y)| (x - cx) * (x - cx) + (y - cy) * (y - cy))
            .map(|(n, _, _)| n.clone())
    };
    let mut tap_node = |c: &crate::def::Comp, untapped: &mut usize| -> Option<String> {
        match taps.get(&c.name) {
            Some(&(tx, ty)) => Some(node(&rail, tx, ty)),
            None => {
                *untapped += 1;
                nearest(c.x, c.y)
            }
        }
    };

    if (!job.power_map.is_empty() || !job.decap_map.is_empty() || !job.current_map.is_empty())
        && !def.comps.is_empty()
        && !lnodes.is_empty()
    {
        // The seam on silicon: each instance's current = its cell's char switching
        // energy, landed on the nearest rail node; the same energy drives a switch
        // event for dynamic IR. Static avg current = (energy/vdd) · f · activity.
        //
        // When a `current_map` is supplied (vyges-power's per-instance activity map),
        // it OVERRIDES that worst-case `activity` assumption: each instance's static
        // current is the measured/estimated value, so the droop reflects real activity
        // instead of worst-case-simultaneous switching. The char energy still drives
        // the per-instance switch event for the dynamic solve.
        let f = job.clock_ghz * 1e9;
        let mut sload: BTreeMap<String, f64> = BTreeMap::new();
        let mut senergy: BTreeMap<String, f64> = BTreeMap::new();
        let mut dcap: BTreeMap<String, f64> = BTreeMap::new();
        for c in &def.comps {
            // static current: per-instance from vyges-power if present, else q·f·activity
            if let Some(&cur) = imap.get(&c.name) {
                if let Some(name) = tap_node(c, &mut untapped) {
                    *sload.entry(name.clone()).or_default() += cur;
                    if let Some(&e) = pmap.get(&c.cell) {
                        if e > 0.0 {
                            *senergy.entry(name.clone()).or_default() += e;
                        }
                    }
                }
            } else if let Some(&e) = pmap.get(&c.cell) {
                if e > 0.0 {
                    if let Some(name) = tap_node(c, &mut untapped) {
                        let q = e * 1e-12 / job.vdd; // Coulombs per switch
                        *sload.entry(name.clone()).or_default() += q * f * job.activity;
                        *senergy.entry(name.clone()).or_default() += e;
                    }
                }
            }
            // placed decoupling capacitance from decap cells
            if let Some(&cf) = dmap.get(&c.cell) {
                if cf > 0.0 {
                    if let Some(name) = tap_node(c, &mut untapped) {
                        *dcap.entry(name.clone()).or_default() += cf;
                    }
                }
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
        // a uniform node_cap_pf (if set) tops up every rail node on top of placed decap.
        if job.node_cap_pf > 0.0 {
            for (n, _, _) in &lnodes {
                *dcap.entry(n.clone()).or_default() += job.node_cap_pf;
            }
        }
        caps = dcap.into_iter().collect();
    } else if job.total_current > 0.0 && !lnodes.is_empty() {
        // uniform fallback: spread the total current over the rail nodes.
        let per = job.total_current / lnodes.len() as f64;
        loads = lnodes.iter().map(|(n, _, _)| (n.clone(), per)).collect();
    }

    Ok(PdnSpec {
        vdd: job.vdd,
        pads,
        resistors,
        loads,
        switches,
        caps,
        ..Default::default()
    })
}
