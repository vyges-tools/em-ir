use vyges_em_ir::solver::LinSys;

// pad(1.8) --0.1ohm--> n1, load 1A at n1  =>  v(n1) = 1.8 - 1*0.1 = 1.7
#[test]
fn single_resistor_drop() {
    let mut sys = LinSys::new(1);
    sys.diag[0] = 10.0; // g = 1/0.1
    sys.rhs[0] = -1.0 + 10.0 * 1.8; // -load + g*v_pad
    let x = sys.solve(10_000, 1e-12).unwrap();
    assert!((x[0] - 1.7).abs() < 1e-6, "v={}", x[0]);
}

// pad --0.1--> n1 --0.1--> n2, load 1A at n2  =>  v(n1)=1.7, v(n2)=1.6
#[test]
fn series_chain() {
    let mut sys = LinSys::new(2);
    sys.diag[0] = 20.0;
    sys.diag[1] = 10.0;
    sys.offdiag[0].push((1, 10.0));
    sys.offdiag[1].push((0, 10.0));
    sys.rhs[0] = 10.0 * 1.8; // pad neighbour
    sys.rhs[1] = -1.0; // load
    let x = sys.solve(10_000, 1e-12).unwrap();
    assert!((x[0] - 1.7).abs() < 1e-6, "v1={}", x[0]);
    assert!((x[1] - 1.6).abs() < 1e-6, "v2={}", x[1]);
}

#[test]
fn singular_floating_node() {
    let sys = LinSys::new(1); // diag stays 0 -> floating
    assert!(sys.solve(100, 1e-9).is_err());
}
