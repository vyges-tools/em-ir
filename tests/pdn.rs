use vyges_em_ir::pdn::PdnSpec;

#[test]
fn parses_network() {
    let s = PdnSpec::parse(
        "vdd 1.8\npad p\nres p n1 0.1 met5\nvia n1 m1 2.0\nload n1 0.5\nemlimit met5 0.8\n",
    )
    .unwrap();
    assert_eq!(s.vdd, 1.8);
    assert_eq!(s.pads, vec![("p".to_string(), 1.8)]); // pad voltage defaults to vdd
    assert_eq!(s.resistors.len(), 2);
    assert_eq!(s.resistors[1].layer.as_deref(), Some("via"));
    assert_eq!(s.loads, vec![("n1".to_string(), 0.5)]);
    assert_eq!(s.em_limits.get("met5"), Some(&0.8));
}

#[test]
fn rejects_missing_essentials() {
    assert!(PdnSpec::parse("pad p\nres p n1 0.1\n").is_err()); // no vdd
    assert!(PdnSpec::parse("vdd 1.8\nres p n1 0.1\n").is_err()); // no pad
    assert!(PdnSpec::parse("vdd 1.8\npad p\n").is_err()); // no resistor
    assert!(PdnSpec::parse("vdd 1.8\npad p\nres p n1 0 met5\n").is_err()); // r <= 0
}
