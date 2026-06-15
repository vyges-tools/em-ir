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
    let spec = extract(&Def::parse(DEF).unwrap(), &TechLef::parse(LEF).unwrap(), &job()).unwrap();
    assert_eq!(spec.resistors.len(), 8, "4 wire + 4 via resistors");
    assert_eq!(spec.pads.len(), 4, "4 met5 corner nodes are pads");
    // every wire stripe is 10um/1um * 0.1 ohm/sq = 1.0 ohm
    let wires: Vec<&f64> =
        spec.resistors.iter().filter(|r| r.layer.as_deref() != Some("via")).map(|r| &r.r).collect();
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
    let spec = extract(&Def::parse(STACK_DEF).unwrap(), &TechLef::parse(LEF).unwrap(), &j).unwrap();
    // met5 stripe (0..20um) split at the via (0,10um) -> two 10um/1um = 1 ohm wires,
    // plus one via resistor met4<->met5 at the mid point.
    let met5: Vec<f64> =
        spec.resistors.iter().filter(|r| r.layer.as_deref() == Some("met5")).map(|r| r.r).collect();
    assert_eq!(met5.len(), 2, "stripe split at the mid-segment via");
    for r in &met5 {
        assert!((r - 1.0).abs() < 1e-9, "each half = 1 ohm, got {r}");
    }
    assert_eq!(
        spec.resistors.iter().filter(|r| r.layer.as_deref() == Some("via")).count(),
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
    let spec = extract(&Def::parse(DEF).unwrap(), &TechLef::parse(LEF).unwrap(), &job()).unwrap();
    let rep = analyze(&spec).unwrap();
    // each met4 node: 1 mA through its 1-ohm via to a vdd pad (the inter-node stripe
    // carries ~0 by symmetry) -> 1 mV droop.
    let w = rep.worst_ir.unwrap();
    assert!((w.drop - 0.001).abs() < 5e-5, "extracted-grid IR drop ~1 mV, got {}", w.drop);
}
