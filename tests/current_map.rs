// current_map: a per-instance current (from vyges-power's activity map) overrides
// the global `q · f · activity` worst-case assumption — each instance's static
// current is the measured/estimated value, landed on its nearest rail node.
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

// met5 pad stripe + met4 rail (nodes at x=0 and x=10um). Two instances, one near
// each rail node.
const DEF: &str = "\
UNITS DISTANCE MICRONS 1000 ;
COMPONENTS 2 ;
- g1 INVX + PLACED ( 1000 100 ) N ;
- g2 INVX + PLACED ( 9000 100 ) N ;
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

fn job(current_map: String, activity: f64, power_map: String) -> EmIrJob {
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
        decap_map: String::new(),
        clock_ghz: 1.0,
        activity,
        switch_t_ns: 1.0,
        switch_dur_ns: 0.1,
        node_cap_pf: 0.0,
        current_map,
        base_dir: String::new(),
    }
}

fn total_load(current_map: &str, activity: f64, power_map: &str) -> f64 {
    let spec = extract(
        &Def::parse(DEF).unwrap(),
        &TechLef::parse(LEF).unwrap(),
        &job(current_map.to_string(), activity, power_map.to_string()),
    )
    .unwrap();
    spec.loads.iter().map(|(_, c)| c).sum()
}

#[test]
fn current_map_lands_per_instance_current_verbatim() {
    let cmap = tmp("emir_cm1.map", "g1 1.0e-3\ng2 2.0e-4\n");
    let spec = extract(
        &Def::parse(DEF).unwrap(),
        &TechLef::parse(LEF).unwrap(),
        &job(cmap, 0.5, String::new()),
    )
    .unwrap();
    // 1 mA at g1's node (met4_0_0), 0.2 mA at g2's (met4_10000_0)
    let at = |n: &str| spec.loads.iter().find(|(node, _)| node == n).map(|(_, c)| *c).unwrap_or(0.0);
    assert!((at("met4_0_0") - 1.0e-3).abs() < 1e-9, "g1 -> 1 mA");
    assert!((at("met4_10000_0") - 2.0e-4).abs() < 1e-9, "g2 -> 0.2 mA");
}

#[test]
fn current_map_overrides_activity() {
    // the same current_map yields the same load regardless of the (now-ignored)
    // global activity factor — the worst-case assumption is replaced.
    let cmap = tmp("emir_cm2.map", "g1 5.0e-4\ng2 5.0e-4\n");
    let a = total_load(&cmap, 0.1, "");
    let b = total_load(&cmap, 0.9, "");
    assert!((a - b).abs() < 1e-12, "activity no longer changes the static current");
    assert!((a - 1.0e-3).abs() < 1e-9, "0.5 mA + 0.5 mA");
}

#[test]
fn measured_map_lower_than_worstcase_map() {
    // vyges-power "measured" currents are below a worst-case-simultaneous map ->
    // strictly lower total injected current (hence lower droop on the same grid).
    let worst = tmp("emir_worst.map", "g1 1.0e-3\ng2 1.0e-3\n");
    let measured = tmp("emir_meas.map", "g1 3.0e-4\ng2 5.0e-5\n");
    let tw = total_load(&worst, 1.0, "");
    let tm = total_load(&measured, 1.0, "");
    assert!(tm < tw, "measured load {tm} < worst-case {tw}");
    assert!(tw / tm > 3.0, "worst-case overpredicts the injected current by >3x here");
}
