// PDN extraction: a synthetic DEF power grid + tech LEF -> the resistor network,
// with hand-checkable resistances and IR drop.
//
// Grid (10 um square, 1 um wide stripes, 0.1 ohm/sq):
//   two vertical met5 stripes (x=0, x=10um), two horizontal met4 stripes (y=0,
//   y=10um), vias at the 4 corners. met5 is the supply (pad) layer.
//   Each 10 um / 1 um stripe = 0.1 * 10 = 1.0 ohm; each via = 1.0 ohm.
use vyges_em_ir::def::Def;
use vyges_em_ir::emir::analyze;
use vyges_em_ir::extract::extract;
use vyges_em_ir::job::EmIrJob;
use vyges_em_ir::lef::TechLef;

const LEF: &str = "\
LAYER met5
  TYPE ROUTING ;
  RESISTANCE RPERSQ 0.1 ;
  WIDTH 1.0 ;
END met5
LAYER met4
  TYPE ROUTING ;
  RESISTANCE RPERSQ 0.1 ;
  WIDTH 1.0 ;
END met4
";

const DEF: &str = "\
UNITS DISTANCE MICRONS 1000 ;
SPECIALNETS 1 ;
- VPWR
  + USE POWER
  + ROUTED met5 1000 ( 0 0 ) M54 ( 0 10000 ) M54
    NEW met5 1000 ( 10000 0 ) M54 ( 10000 10000 ) M54
    NEW met4 1000 ( 0 0 ) ( 10000 0 )
    NEW met4 1000 ( 0 10000 ) ( 10000 10000 )
 ;
END SPECIALNETS
";

fn job() -> EmIrJob {
    EmIrJob {
        design: "grid".into(),
        pdn: String::new(),
        ir_limit_pct: 5.0,
        def: "(test)".into(),
        cell_lef: String::new(),
        lef: "(test)".into(),
        vdd: 1.8,
        pad_layer: "met5".into(),
        via_res: 1.0,
        total_current: 0.004, // 4 mA over the 4 met4 nodes -> 1 mA each
        power_map: String::new(),
        decap_map: String::new(),
        clock_ghz: 1.0,
        activity: 0.2,
        switch_t_ns: 1.0,
        switch_dur_ns: 0.1,
        node_cap_pf: 0.0,
        current_map: String::new(),
        base_dir: String::new(),
    }
}

#[test]
fn parses_def_units_and_power_net() {
    let def = Def::parse(DEF).unwrap();
    assert_eq!(def.dbu, 1000.0);
    let net = def.power_net().unwrap();
    assert_eq!(net.name, "VPWR");
    assert!(net.use_power);
    assert_eq!(net.segs.len(), 4, "two met5 + two met4 stripes");
    assert_eq!(net.vias.len(), 4, "a via at each corner");
}

#[test]
fn lef_layer_resistance() {
    let lef = TechLef::parse(LEF).unwrap();
    assert!((lef.layers["met5"].rpersq - 0.1).abs() < 1e-12);
    assert!((lef.layers["met4"].width_um - 1.0).abs() < 1e-12);
}

#[test]
fn extracts_network_with_correct_resistances() {
    let spec = extract(
        &Def::parse(DEF).unwrap(),
        &TechLef::parse(LEF).unwrap(),
        &job(),
    )
    .unwrap();
    assert_eq!(spec.resistors.len(), 8, "4 wire + 4 via resistors");
    assert_eq!(spec.pads.len(), 4, "4 met5 corner nodes are pads");
    // every wire stripe is 10um/1um * 0.1 ohm/sq = 1.0 ohm
    let wires: Vec<&f64> = spec
        .resistors
        .iter()
        .filter(|r| r.layer.as_deref() != Some("via"))
        .map(|r| &r.r)
        .collect();
    assert_eq!(wires.len(), 4);
    for r in wires {
        assert!((r - 1.0).abs() < 1e-9, "stripe R = 1.0 ohm, got {r}");
    }
    // 4 mA total over 4 met4 nodes
    assert_eq!(spec.loads.len(), 4);
    assert!((spec.loads.iter().map(|(_, i)| i).sum::<f64>() - 0.004).abs() < 1e-12);
}

// A via landing in the MIDDLE of a stripe, with a single-point landing on the layer
// below — the via-stack case the counter exposed. The stripe must split at the via
// and the via must bridge the two layers.
const STACK_DEF: &str = "\
UNITS DISTANCE MICRONS 1000 ;
SPECIALNETS 1 ;
- VPWR
  + USE POWER
  + ROUTED met5 1000 ( 0 0 ) ( 0 20000 )
    NEW met4 0 ( 0 10000 ) M45
 ;
END SPECIALNETS
";

#[test]
fn parses_components_placements() {
    let def_with_comps = format!(
        "COMPONENTS 2 ;\n\
         - g1 sky130_fd_sc_hd__inv_2 + PLACED ( 5000 6000 ) N ;\n\
         - FILLER_0 sky130_fd_sc_hd__fill_1 + SOURCE DIST + PLACED ( 100 200 ) FS ;\n\
         END COMPONENTS\n{DEF}"
    );
    let def = Def::parse(&def_with_comps).unwrap();
    assert_eq!(def.comps.len(), 2);
    assert_eq!(def.comps[0].name, "g1");
    assert_eq!(def.comps[0].cell, "sky130_fd_sc_hd__inv_2");
    assert_eq!((def.comps[0].x, def.comps[0].y), (5000, 6000));
    assert_eq!(def.comps[1].cell, "sky130_fd_sc_hd__fill_1");
}

#[test]
fn splits_stripe_at_mid_segment_via_and_bridges() {
    let mut j = job();
    j.total_current = 0.001;
    let spec = extract(
        &Def::parse(STACK_DEF).unwrap(),
        &TechLef::parse(LEF).unwrap(),
        &j,
    )
    .unwrap();
    // met5 stripe (0..20um) split at the via (0,10um) -> two 10um/1um = 1 ohm wires,
    // plus one via resistor met4<->met5 at the mid point.
    let met5: Vec<f64> = spec
        .resistors
        .iter()
        .filter(|r| r.layer.as_deref() == Some("met5"))
        .map(|r| r.r)
        .collect();
    assert_eq!(met5.len(), 2, "stripe split at the mid-segment via");
    for r in &met5 {
        assert!((r - 1.0).abs() < 1e-9, "each half = 1 ohm, got {r}");
    }
    assert_eq!(
        spec.resistors
            .iter()
            .filter(|r| r.layer.as_deref() == Some("via"))
            .count(),
        1,
        "one via bridges met4<->met5 at the landing"
    );
    // the bridge connects the split-point met5 node and the met4 landing node
    assert!(spec
        .resistors
        .iter()
        .any(|r| r.layer.as_deref() == Some("via")
            && [r.a.as_str(), r.b.as_str()].contains(&"met5_0_10000")
            && [r.a.as_str(), r.b.as_str()].contains(&"met4_0_10000")));
}

#[test]
fn solves_ir_on_extracted_grid() {
    let spec = extract(
        &Def::parse(DEF).unwrap(),
        &TechLef::parse(LEF).unwrap(),
        &job(),
    )
    .unwrap();
    let rep = analyze(&spec).unwrap();
    // each met4 node: 1 mA through its 1-ohm via to a vdd pad (the inter-node stripe
    // carries ~0 by symmetry) -> 1 mV droop.
    let w = rep.worst_ir.unwrap();
    assert!(
        (w.drop - 0.001).abs() < 5e-5,
        "extracted-grid IR drop ~1 mV, got {}",
        w.drop
    );
}

// ── per-via resistance, from the via definition rather than one flat number ──────────

/// A via is priced by its own cut layer and cut count, not by a single constant.
///
/// Both facts live in the DEF `VIAS` entry and nowhere else: `LAYERS <below> <cut> <above>`
/// names the cut layer whose LEF `RESISTANCE` is stated **per cut**, and `ROWCOL` gives how
/// many cuts are in the array. The via's own name is deliberately misleading here — exactly as
/// sky130 writes it — so a reader that parses the name instead of the definition fails.
const VIA_LEF: &str = "\
LAYER met5
  TYPE ROUTING ;
  RESISTANCE RPERSQ 0.1 ;
  WIDTH 1.0 ;
END met5
LAYER met4
  TYPE ROUTING ;
  RESISTANCE RPERSQ 0.1 ;
  WIDTH 1.0 ;
END met4
LAYER via4
  TYPE CUT ;
  RESISTANCE 0.38 ;
END via4
";

const VIA_DEF: &str = "\
UNITS DISTANCE MICRONS 1000 ;
VIAS 1 ;
    - via5_6_named_to_mislead + VIARULE M4M5_PR + LAYERS met4 via4 met5  + ROWCOL 1 4  ;
END VIAS
SPECIALNETS 1 ;
- VPWR
  + USE POWER
  + ROUTED met5 1000 ( 0 0 ) ( 0 10000 )
    NEW met4 1000 ( 0 0 ) ( 10000 0 )
    NEW met4 1000 ( 0 0 ) 0 via5_6_named_to_mislead
 ;
END SPECIALNETS
";

#[test]
fn a_via_is_priced_by_its_cut_layer_and_cut_count() {
    let def = Def::parse(VIA_DEF).unwrap();
    let lef = TechLef::parse(VIA_LEF).unwrap();
    let mut j = job();
    j.via_res = 5.0; // the old flat value — must NOT be what the via ends up costing
    let spec = extract(&def, &lef, &j).unwrap();

    let vias: Vec<_> = spec
        .resistors
        .iter()
        .filter(|r| r.layer.as_deref() == Some("via"))
        .collect();
    assert_eq!(vias.len(), 1, "one via bridging met4 and met5");

    // 0.38 ohm per cut over a 1x4 array = 0.095 ohm, not the 5.0 ohm fallback and not 0.38.
    let expected = 0.38 / 4.0;
    assert!(
        (vias[0].r - expected).abs() < 1e-12,
        "via priced at {} ohm; expected {} = per-cut 0.38 / 4 cuts (flat fallback would give {})",
        vias[0].r,
        expected,
        j.via_res
    );
}

#[test]
fn a_via_with_no_definition_falls_back_rather_than_vanishing() {
    // The same grid with the VIAS section removed: nothing states a cut layer or a cut count,
    // so the job's via_res stands in. A via that silently disappeared would break the stack
    // and leave the lower layers unreachable — a much worse failure than an approximate value.
    let no_defs = VIA_DEF.replace(
        "VIAS 1 ;\n    - via5_6_named_to_mislead + VIARULE M4M5_PR + LAYERS met4 via4 met5  + ROWCOL 1 4  ;\nEND VIAS\n",
        "",
    );
    let def = Def::parse(&no_defs).unwrap();
    let lef = TechLef::parse(VIA_LEF).unwrap();
    let mut j = job();
    j.via_res = 5.0;
    let spec = extract(&def, &lef, &j).unwrap();

    let vias: Vec<_> = spec
        .resistors
        .iter()
        .filter(|r| r.layer.as_deref() == Some("via"))
        .collect();
    assert_eq!(vias.len(), 1, "the via is still built");
    assert!(
        (vias[0].r - 5.0).abs() < 1e-12,
        "with no definition to price it, the via falls back to via_res"
    );
}

// ── supply sources: the design's own power pin, not the whole top layer ──────────────

/// Sources come from the power PIN's port shapes where the design declares one.
///
/// The grid is the same two met5 stripes as `DEF`, but only ONE of them carries the pin. The
/// fallback model — every node on `pad_layer` is ideal — would hold all four met5 nodes at vdd
/// and delete the resistance the supply crosses to reach the far stripe, which is why it
/// under-reports IR drop. PDNSim uses the bterms first for exactly this reason.
const PIN_DEF: &str = "\
UNITS DISTANCE MICRONS 1000 ;
PINS 1 ;
    - VPWR + NET VPWR + SPECIAL + DIRECTION INOUT + USE POWER
      + PORT
        + LAYER met5 ( -100 -100 ) ( 100 10100 )
      + FIXED ( 0 0 ) N ;
END PINS
SPECIALNETS 1 ;
- VPWR
  + USE POWER
  + ROUTED met5 1000 ( 0 0 ) M54 ( 0 10000 ) M54
    NEW met5 1000 ( 10000 0 ) M54 ( 10000 10000 ) M54
    NEW met4 1000 ( 0 0 ) ( 10000 0 )
    NEW met4 1000 ( 0 10000 ) ( 10000 10000 )
 ;
END SPECIALNETS
";

#[test]
fn sources_are_the_power_pin_shapes_when_the_design_declares_them() {
    let def = Def::parse(PIN_DEF).unwrap();
    let lef = TechLef::parse(LEF).unwrap();
    let spec = extract(&def, &lef, &job()).unwrap();

    // The port covers x in [-100, 100], so only the x=0 stripe's two nodes are sources —
    // not the x=10000 stripe, which the supply must now reach through met4.
    assert_eq!(
        spec.pads.len(),
        2,
        "only the met5 nodes inside the port rectangle are held at vdd, got {:?}",
        spec.pads.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
    assert!(
        spec.pads.iter().all(|(n, _)| n.starts_with("met5_0_")),
        "the far stripe is NOT a source: {:?}",
        spec.pads
    );
}

#[test]
fn sources_fall_back_to_the_pad_layer_when_no_power_pin_is_declared() {
    // The same grid with no PINS section — PDNSim's own last resort, and ours.
    let def = Def::parse(DEF).unwrap();
    let lef = TechLef::parse(LEF).unwrap();
    let spec = extract(&def, &lef, &job()).unwrap();
    assert_eq!(
        spec.pads.len(),
        4,
        "with nothing declared, every pad_layer node is a source"
    );
}

#[test]
fn a_restricted_source_set_reports_more_ir_drop_than_the_whole_layer() {
    // The direction of the error, pinned. Holding a whole layer ideal removes resistance the
    // real supply has to cross, so the fallback is OPTIMISTIC — and a checker that
    // under-reports blesses designs it should fail.
    let lef = TechLef::parse(LEF).unwrap();
    let whole_layer = analyze(&extract(&Def::parse(DEF).unwrap(), &lef, &job()).unwrap()).unwrap();
    let pin_only = analyze(&extract(&Def::parse(PIN_DEF).unwrap(), &lef, &job()).unwrap()).unwrap();

    let a = whole_layer.worst_ir.unwrap().drop;
    let b = pin_only.worst_ir.unwrap().drop;
    assert!(
        b > a,
        "restricting sources to the declared pin must not reduce IR drop: pin {b} vs layer {a}"
    );
}

// ── a cell taps its rail where the cell sits, not where DEF anchors it ───────────────

/// DEF places an instance by its ORIGIN — the lower-left corner. The cell draws supply
/// through a rail-spanning pin, so current enters around the cell's middle. Using the
/// origin displaces every load by half a cell width.
///
/// Measured against PDNSim on a block of wide cells: `dfstp_2` is 9.66 um, so its supply
/// was entering the rail 4.83 um upstream of where PDNSim injects it, and the along-rail
/// IR drop came out ~10% low as a result.
const CELL_LEF: &str = "\
MACRO wide_cell
  CLASS CORE ;
  SIZE 10.000000 BY 2.720000 ;
  PIN VPWR
    DIRECTION INOUT ;
  END VPWR
END wide_cell
";

const WIDE_DEF: &str = "\
UNITS DISTANCE MICRONS 1000 ;
COMPONENTS 1 ;
- g1 wide_cell + PLACED ( 20000 100 ) N ;
END COMPONENTS
SPECIALNETS 1 ;
- VPWR
  + USE POWER
  + ROUTED met5 1000 ( 0 0 ) M54 ( 0 10000 ) M54
    NEW met4 1000 ( 0 0 ) ( 60000 0 )
 ;
END SPECIALNETS
";

fn wide_job(cell_lef: String) -> EmIrJob {
    let mut j = job();
    j.total_current = 0.0;
    j.cell_lef = cell_lef;
    j.current_map = {
        let p = std::env::temp_dir().join("emir_wide_cur.map");
        std::fs::write(&p, "g1 1.0e-3\n").unwrap();
        p.to_string_lossy().into_owned()
    };
    j
}

#[test]
fn a_cell_taps_its_rail_at_its_centre_when_the_footprint_is_known() {
    let p = std::env::temp_dir().join("emir_cells.lef");
    std::fs::write(&p, CELL_LEF).unwrap();
    let spec = extract(
        &Def::parse(WIDE_DEF).unwrap(),
        &TechLef::parse(LEF).unwrap(),
        &wide_job(p.to_string_lossy().into_owned()),
    )
    .unwrap();

    // origin x = 20000, cell is 10 um wide -> the tap belongs at 25000, not 20000.
    assert!(
        spec.loads.iter().any(|(n, _)| n == "met4_25000_0"),
        "current must enter at the cell's centre; loads were {:?}",
        spec.loads.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
    assert!(
        !spec.loads.iter().any(|(n, _)| n == "met4_20000_0"),
        "and NOT at the DEF origin, which is half a cell width upstream"
    );
}

#[test]
fn without_a_cell_lef_the_tap_falls_back_to_the_origin() {
    // No footprint stated, so there is nothing to centre on. Falling back is right; silently
    // pretending to know the geometry would not be.
    let spec = extract(
        &Def::parse(WIDE_DEF).unwrap(),
        &TechLef::parse(LEF).unwrap(),
        &wide_job(String::new()),
    )
    .unwrap();
    assert!(
        spec.loads.iter().any(|(n, _)| n == "met4_20000_0"),
        "falls back to the DEF origin, got {:?}",
        spec.loads.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
}
