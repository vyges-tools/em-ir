// Dynamic (transient) IR: a `switch` event injects a current pulse whose charge is
// energy/vdd (the char internal_power seam); the backward-Euler solve reports the
// deepest droop, which exceeds the static IR and is smoothed by decap.
use vyges_em_ir::emir::analyze;
use vyges_em_ir::pdn::PdnSpec;

#[test]
fn parses_cap_and_switch() {
    let s =
        PdnSpec::parse("vdd 1.8\npad p\nres p n1 0.5 met1\ncap n1 0.5\nswitch n1 1.8 1.0 0.1\n")
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
    assert!(
        rep.dynamic.is_none(),
        "no transient without switching events"
    );
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
    assert!(
        (d.drop - 0.01).abs() < 2e-3,
        "peak droop ~ ipk*R = 0.01 V, got {}",
        d.drop
    );
    assert!(
        (d.time_ns - 1.0).abs() < 0.05,
        "worst droop near the switch peak (1 ns)"
    );
    assert_eq!(d.node, "n1");
}

#[test]
fn decap_reduces_droop() {
    let no_cap = "vdd 1.8\npad p\nres p n1 0.5\nswitch n1 1.8 1.0 0.1\n";
    let with_cap = "vdd 1.8\npad p\nres p n1 0.5\ncap n1 100\nswitch n1 1.8 1.0 0.1\n";
    let d0 = analyze(&PdnSpec::parse(no_cap).unwrap())
        .unwrap()
        .dynamic
        .unwrap()
        .drop;
    let d1 = analyze(&PdnSpec::parse(with_cap).unwrap())
        .unwrap()
        .dynamic
        .unwrap()
        .drop;
    assert!(
        d1 < d0,
        "a large decap should reduce the dynamic droop ({d1} < {d0})"
    );
}

// ── validating a solve that has no external oracle ───────────────────────────────────
//
// PDNSim is static-only, so there is no second implementation of dynamic IR to correlate
// against and none of these can be a correlation. What is available instead is exact
// mathematics on hand-solvable cases, and invariants that hold by construction on any
// case at all — the same substitutes the reader-hardening campaign used where no golden
// existed. They are weaker than an oracle at finding a *modelling* omission and stronger
// at finding an arithmetic or integrator error, which is the failure mode here.

/// With **no capacitance the transient is exactly quasi-static**: every timestep is an
/// independent DC solve at that instant's current, so the peak droop is `ipk * R` with no
/// integration error at all.
///
/// This is the one case where backward Euler must be exact rather than approximate, which
/// makes it a real check on the assembly (`C/dt` on the diagonal, `i(t)` in the rhs) rather
/// than a plausibility bound. The pre-existing version of this asserted only 20%.
#[test]
fn with_no_decap_the_peak_droop_is_exactly_ipk_times_r() {
    // Q = energy/vdd = 1.8 pJ / 1.8 V = 1e-12 C; a triangle of duration 0.1 ns carrying Q
    // peaks at ipk = 2Q/dur = 0.02 A; through 0.5 ohm that is exactly 0.01 V.
    let s = PdnSpec::parse("vdd 1.8\npad p\nres p n1 0.5\nswitch n1 1.8 1.0 0.1\n").unwrap();
    let d = analyze(&s).unwrap().dynamic.expect("dynamic ran");
    assert!(
        (d.drop - 0.01).abs() < 1e-9,
        "C=0 makes this exact: expected 0.01 V, got {}",
        d.drop
    );
    assert!(
        (d.time_ns - 1.0).abs() < 1e-9,
        "and the peak is exactly at t50, got {}",
        d.time_ns
    );
}

/// The network is linear and time-invariant, so **scaling every switch energy by k scales
/// the droop below the static baseline by exactly k**.
///
/// Holds with capacitance present, which the quasi-static case cannot check — it exercises
/// the `C/dt` term and the previous-step coupling, and it needs no closed-form solution.
///
/// What it does **not** catch, established by mutation rather than assumed: a wrong constant
/// in the charge. Computing `q` as `energy` instead of `energy/vdd` leaves every droop scaled
/// by 1.8 and the *ratio* untouched, so this test passes on that mutant while
/// `with_no_decap_the_peak_droop_is_exactly_ipk_times_r` fails. An invariant on a ratio can
/// only see a nonlinearity; it takes an absolute case to pin a coefficient. The two are here
/// for that reason and neither replaces the other.
#[test]
fn droop_scales_linearly_with_switch_energy() {
    let case = |e: f64| {
        let src = format!("vdd 1.8\npad p\nres p n1 0.5\ncap n1 50\nswitch n1 {e} 1.0 0.1\n");
        let r = analyze(&PdnSpec::parse(&src).unwrap()).unwrap();
        let stat = r.worst_ir.as_ref().unwrap().voltage;
        stat - r.dynamic.unwrap().voltage // droop below the static baseline
    };
    let d1 = case(1.0);
    let d3 = case(3.0);
    assert!(d1 > 0.0, "a switch must produce some droop");
    assert!(
        (d3 / d1 - 3.0).abs() < 1e-6,
        "3x the energy must give exactly 3x the droop, got {}",
        d3 / d1
    );
}

/// **Two switches of energy e on one node are one switch of energy 2e.** Superposition, and
/// it is checked on the composed path rather than on the solver, so it catches a switch
/// accumulated wrongly (overwritten instead of summed) as well as an integration error.
#[test]
fn coincident_switches_superpose() {
    let two = "vdd 1.8\npad p\nres p n1 0.5\ncap n1 50\nswitch n1 1.0 1.0 0.1\nswitch n1 1.0 1.0 0.1\n";
    let one = "vdd 1.8\npad p\nres p n1 0.5\ncap n1 50\nswitch n1 2.0 1.0 0.1\n";
    let d = |src: &str| {
        analyze(&PdnSpec::parse(src).unwrap())
            .unwrap()
            .dynamic
            .unwrap()
            .drop
    };
    let (a, b) = (d(two), d(one));
    assert!(
        (a - b).abs() < 1e-9,
        "two 1 pJ switches must equal one 2 pJ switch: {a} vs {b}"
    );
}

/// **More decoupling capacitance never deepens the droop, and enough of it removes it.**
///
/// Monotonicity is the physical statement; the asymptote is what makes the test sharp. A
/// decap large against the pulse (Q/dV) supplies the charge locally, so the droop tends to
/// zero rather than to some floor — a sign error on the `C/dt` term passes a
/// merely-monotonic check and fails this one.
#[test]
fn decap_monotonically_removes_droop() {
    let d = |cap_pf: f64| {
        let src =
            format!("vdd 1.8\npad p\nres p n1 0.5\ncap n1 {cap_pf}\nswitch n1 1.8 1.0 0.1\n");
        analyze(&PdnSpec::parse(&src).unwrap())
            .unwrap()
            .dynamic
            .unwrap()
            .drop
    };
    let series: Vec<f64> = [0.0, 10.0, 100.0, 1_000.0, 100_000.0].iter().map(|c| d(*c)).collect();
    for w in series.windows(2) {
        assert!(
            w[1] <= w[0] + 1e-12,
            "droop must not grow with added decap: {:?}",
            series
        );
    }
    assert!(
        series[4] < series[0] / 100.0,
        "a decap 1e5 pF against a 1 pC pulse should all but remove the droop, got {:?}",
        series
    );
}
