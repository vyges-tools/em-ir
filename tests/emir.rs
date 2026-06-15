use vyges_em_ir::emir::analyze;
use vyges_em_ir::pdn::PdnSpec;

#[test]
fn ir_drop_and_em_violation() {
    // pad(1.8) --0.1ohm,met5--> n1, load 1A; met5 limit 0.8A
    let spec =
        PdnSpec::parse("vdd 1.8\npad p\nres p n1 0.1 met5\nload n1 1.0\nemlimit met5 0.8\n").unwrap();
    let rep = analyze(&spec).unwrap();

    let w = rep.worst_ir.unwrap();
    assert_eq!(w.node, "n1");
    assert!((w.drop - 0.1).abs() < 1e-6, "drop={}", w.drop); // 1A * 0.1ohm
    assert!((w.drop_pct - 5.5556).abs() < 1e-3, "pct={}", w.drop_pct);

    // current 1.0A vs limit 0.8A -> ratio 1.25, one violation
    assert_eq!(rep.em_checked, 1);
    assert_eq!(rep.em_violations.len(), 1);
    assert!((rep.em_worst_ratio - 1.25).abs() < 1e-6, "ratio={}", rep.em_worst_ratio);
}

#[test]
fn clean_pdn_passes() {
    // small load, generous limits -> no violations, tiny drop
    let spec = PdnSpec::parse("vdd 1.8\npad p\nres p n1 0.1 met5\nload n1 0.01\nemlimit met5 1.0\n")
        .unwrap();
    let rep = analyze(&spec).unwrap();
    assert_eq!(rep.em_violations.len(), 0);
    assert!(rep.worst_ir.unwrap().drop_pct < 1.0);
}
