// Decap extraction: a decap cell in DEF COMPONENTS lands its capacitance (from a
// cell->pF decap_map) on the nearest supply-rail node — the placed decoupling that
// smooths the dynamic droop, alongside the char-energy switching current.
use std::fs;
use vyges_em_ir::def::Def;
use vyges_em_ir::extract::extract;
use vyges_em_ir::job::EmIrJob;
use vyges_em_ir::lef::TechLef;

const LEF: &str = "\
LAYER met5
  RESISTANCE RPERSQ 0.1 ;
  WIDTH 1.0 ;
END met5
LAYER met4
  RESISTANCE RPERSQ 0.1 ;
  WIDTH 1.0 ;
END met4
";

// met5 pad stripe + met4 rail (two nodes at x=0 and x=10um), via at (0,0).
// Components: an inverter near the (0,0) rail node, a decap near the (10um,0) node.
const DEF: &str = "\
UNITS DISTANCE MICRONS 1000 ;
COMPONENTS 2 ;
- g1 INVX + PLACED ( 1000 100 ) N ;
- dcap0 DECAP + SOURCE DIST + PLACED ( 9000 100 ) N ;
END COMPONENTS
SPECIALNETS 1 ;
- VPWR
  + USE POWER
  + ROUTED met5 1000 ( 0 0 ) M ( 0 10000 )
    NEW met4 1000 ( 0 0 ) ( 10000 0 )
 ;
END SPECIALNETS
";

fn tmp(name: &str, content: &str) -> String {
    let p = std::env::temp_dir().join(name);
    fs::write(&p, content).unwrap();
    p.to_string_lossy().into_owned()
}

fn job(power_map: String, decap_map: String) -> EmIrJob {
    EmIrJob {
        design: "d".into(),
        pdn: String::new(),
        ir_limit_pct: 25.0,
        def: "(test)".into(),
        lef: "(test)".into(),
        vdd: 1.8,
        pad_layer: "met5".into(),
        via_res: 1.0,
        total_current: 0.0,
        power_map,
        decap_map,
        clock_ghz: 1.0,
        activity: 0.3,
        switch_t_ns: 1.0,
        switch_dur_ns: 0.1,
        node_cap_pf: 0.0,
        current_map: String::new(),
        base_dir: String::new(), // absolute tmp paths -> resolve passes them through
    }
}

#[test]
fn decap_cell_lands_capacitance_at_its_own_tap_on_the_rail() {
    let pmap = tmp("emir_pwr.map", "INVX 0.02\n");
    let dmap = tmp("emir_decap.map", "DECAP 0.5\n");
    let spec = extract(
        &Def::parse(DEF).unwrap(),
        &TechLef::parse(LEF).unwrap(),
        &job(pmap, dmap),
    )
    .unwrap();
    // The decap sits at x=9um, so its capacitance belongs on the rail beneath it — not at
    // the rail's far end. Where decoupling capacitance sits is the whole point of placing
    // it: moved to an endpoint it decouples somewhere the current never flows.
    assert_eq!(spec.caps.len(), 1, "one placed decap");
    assert_eq!(spec.caps[0].0, "met4_9000_0");
    assert!((spec.caps[0].1 - 0.5).abs() < 1e-12, "0.5 pF placed");
    // likewise the inverter at x=1um draws current and switches at its own tap
    assert!(spec.loads.iter().any(|(n, _)| n == "met4_1000_0"));
    assert_eq!(spec.switches.len(), 1);
    assert_eq!(spec.switches[0].node, "met4_1000_0");
    // and nothing is left at the endpoints the old nearest-node rule used
    assert!(!spec.loads.iter().any(|(n, _)| n == "met4_0_0"));
    assert!(!spec.caps.iter().any(|(n, _)| n == "met4_10000_0"));
}

#[test]
fn no_decap_map_means_no_caps() {
    let pmap = tmp("emir_pwr2.map", "INVX 0.02\n");
    let spec = extract(
        &Def::parse(DEF).unwrap(),
        &TechLef::parse(LEF).unwrap(),
        &job(pmap, String::new()),
    )
    .unwrap();
    assert!(
        spec.caps.is_empty(),
        "no decap_map -> no placed capacitance"
    );
}
