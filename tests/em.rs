// Real EM from LEF current-density: a per-segment limit = DC current-density (mA/um)
// × wire width, and a current over it is flagged with its ratio.
use vyges_em_ir::def::Def;
use vyges_em_ir::emir::analyze;
use vyges_em_ir::extract::extract;
use vyges_em_ir::job::EmIrJob;
use vyges_em_ir::lef::TechLef;
use vyges_em_ir::pdn::{PdnSpec, Resistor, Switch};

const LEF: &str = "\
LAYER met5
  RESISTANCE RPERSQ 0.1 ;
  WIDTH 1.0 ;
  DCCURRENTDENSITY AVERAGE 2.0 ;
END met5
LAYER met4
  RESISTANCE RPERSQ 0.1 ;
  WIDTH 1.0 ;
  DCCURRENTDENSITY AVERAGE 1.0 ;
END met4
";

const DEF: &str = "\
UNITS DISTANCE MICRONS 1000 ;
SPECIALNETS 1 ;
- VPWR
  + USE POWER
  + ROUTED met5 1000 ( 0 0 ) M ( 0 10000 )
    NEW met4 1000 ( 0 0 ) ( 10000 0 )
 ;
END SPECIALNETS
";

fn job() -> EmIrJob {
    EmIrJob {
        design: "em".into(),
        pdn: String::new(),
        ir_limit_pct: 25.0,
        def: "(t)".into(),
        lef: "(t)".into(),
        vdd: 1.8,
        pad_layer: "met5".into(),
        via_res: 1.0,
        total_current: 0.0,
        power_map: String::new(),
        decap_map: String::new(),
        clock_ghz: 1.0,
        activity: 0.3,
        switch_t_ns: 1.0,
        switch_dur_ns: 0.1,
        node_cap_pf: 0.0,
        current_map: String::new(),
        base_dir: String::new(),
    }
}

#[test]
fn lef_dc_current_density_parsed() {
    let lef = TechLef::parse(LEF).unwrap();
    assert!((lef.layers["met4"].dc_jmax - 1.0).abs() < 1e-12);
    assert!((lef.layers["met5"].dc_jmax - 2.0).abs() < 1e-12);
}

#[test]
fn extract_sets_width_based_em_limit() {
    let spec = extract(
        &Def::parse(DEF).unwrap(),
        &TechLef::parse(LEF).unwrap(),
        &job(),
    )
    .unwrap();
    // met4 wire: 1.0 mA/um × 1.0 um = 1.0 mA = 0.001 A
    let met4 = spec
        .resistors
        .iter()
        .find(|r| r.layer.as_deref() == Some("met4"))
        .unwrap();
    assert!(
        (met4.em_limit.unwrap() - 0.001).abs() < 1e-12,
        "limit = jmax·width"
    );
}

#[test]
fn per_segment_em_violation_flagged() {
    // a 0.001 A limit segment carrying 0.002 A -> ratio 2.0 -> violation.
    let spec = PdnSpec {
        vdd: 1.8,
        pads: vec![("p".into(), 1.8)],
        resistors: vec![Resistor {
            a: "p".into(),
            b: "n1".into(),
            r: 1.0,
            layer: Some("met1".into()),
            em_limit: Some(0.001),
            em_rms_limit: None,
            em_peak_limit: None,
        }],
        loads: vec![("n1".into(), 0.002)],
        ..Default::default()
    };
    let rep = analyze(&spec).unwrap();
    assert_eq!(rep.em_checked, 1);
    assert_eq!(rep.em_violations.len(), 1);
    assert!(
        (rep.em_violations[0].ratio - 2.0).abs() < 1e-6,
        "2 mA / 1 mA = 2x"
    );
    assert!((rep.em_worst_ratio - 2.0).abs() < 1e-6);
}

fn rms_spec(rms_limit: f64) -> PdnSpec {
    PdnSpec {
        vdd: 1.8,
        pads: vec![("p".into(), 1.8)],
        resistors: vec![Resistor {
            a: "p".into(),
            b: "n1".into(),
            r: 1.0,
            layer: Some("met1".into()),
            em_limit: None,
            em_rms_limit: Some(rms_limit),
            em_peak_limit: None,
        }],
        // a switching pulse draws current through the segment for the transient
        switches: vec![Switch {
            node: "n1".into(),
            energy_pj: 1.0,
            t50_ns: 1.0,
            dur_ns: 0.1,
        }],
        ..Default::default()
    }
}

#[test]
fn rms_em_from_transient_flags_over_limit() {
    // a tiny RMS limit -> the segment's transient RMS current is over it (kind "rms")
    let rep = analyze(&rms_spec(0.0002)).unwrap();
    assert!(rep.dynamic.is_some(), "transient ran");
    let rms: Vec<_> = rep
        .em_violations
        .iter()
        .filter(|v| v.kind == "rms")
        .collect();
    assert_eq!(rms.len(), 1, "RMS current over the limit");
    assert!(rms[0].ratio > 1.0);
    // a generous limit -> no RMS violation
    let rep2 = analyze(&rms_spec(0.05)).unwrap();
    assert!(
        rep2.em_violations.iter().all(|v| v.kind != "rms"),
        "RMS met with a large limit"
    );
}
