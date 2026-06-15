// Dynamic (transient) IR: a `switch` event injects a current pulse whose charge is
// energy/vdd (the char internal_power seam); the backward-Euler solve reports the
// deepest droop, which exceeds the static IR and is smoothed by decap.
use vyges_em_ir::emir::analyze;
use vyges_em_ir::pdn::PdnSpec;

#[test]
fn parses_cap_and_switch() {
    let s = PdnSpec::parse(
        "vdd 1.8\npad p\nres p n1 0.5 met1\ncap n1 0.5\nswitch n1 1.8 1.0 0.1\n",
    )
    .unwrap();
    assert!(s.is_dynamic());
    assert_eq!(s.caps, vec![("n1".to_string(), 0.5)]);
    assert_eq!(s.switches.len(), 1);
    let sw = &s.switches[0];
    assert_eq!(sw.node, "n1");
    assert!((sw.energy_pj - 1.8).abs() < 1e-12 && (sw.t50_ns - 1.0).abs() < 1e-12);
    assert!((sw.dur_ns - 0.1).abs() < 1e-12);
}

#[test]
fn no_switches_means_static_only() {
    let s = PdnSpec::parse("vdd 1.8\npad p\nres p n1 0.5\nload n1 0.1\n").unwrap();
    assert!(!s.is_dynamic());
    let rep = analyze(&s).unwrap();
    assert!(rep.dynamic.is_none(), "no transient without switching events");
}

#[test]
fn dynamic_droop_present_and_exceeds_static() {
    // No static load -> static IR is ~0; a switching pulse drives the only droop.
    // Q = energy/vdd = 1.8pJ/1.8V = 1e-12 C; over a 0.1ns triangle, ipk = 2Q/dur =
    // 0.02 A; through 0.5 ohm the peak droop ~ 0.01 V (no decap -> quasi-static).
    let s = PdnSpec::parse("vdd 1.8\npad p\nres p n1 0.5\nswitch n1 1.8 1.0 0.1\n").unwrap();
    let rep = analyze(&s).unwrap();
    let stat = rep.worst_ir.as_ref().unwrap().drop;
    assert!(stat < 1e-6, "no static load -> ~0 static drop, got {stat}");
    let d = rep.dynamic.expect("dynamic analysis ran");
    assert!(d.drop > stat, "dynamic droop must exceed static");
    assert!((d.drop - 0.01).abs() < 2e-3, "peak droop ~ ipk*R = 0.01 V, got {}", d.drop);
    assert!((d.time_ns - 1.0).abs() < 0.05, "worst droop near the switch peak (1 ns)");
    assert_eq!(d.node, "n1");
}

#[test]
fn decap_reduces_droop() {
    let no_cap = "vdd 1.8\npad p\nres p n1 0.5\nswitch n1 1.8 1.0 0.1\n";
    let with_cap = "vdd 1.8\npad p\nres p n1 0.5\ncap n1 100\nswitch n1 1.8 1.0 0.1\n";
    let d0 = analyze(&PdnSpec::parse(no_cap).unwrap()).unwrap().dynamic.unwrap().drop;
    let d1 = analyze(&PdnSpec::parse(with_cap).unwrap()).unwrap().dynamic.unwrap().drop;
    assert!(d1 < d0, "a large decap should reduce the dynamic droop ({d1} < {d0})");
}
