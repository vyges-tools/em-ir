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

// ── convergence on a mesh the size of a real PDN ─────────────────────────────────────

/// Build an `n × n` resistive mesh: unit conductance between orthogonal neighbours, the whole
/// left column tied to a 1.8 V supply, and a unit current drawn from the far corner.
///
/// Returns the system plus the node index of that corner.
fn mesh(n: usize, g: f64, vdd: f64, load: f64) -> (LinSys, usize) {
    let idx = |i: usize, j: usize| i * n + j;
    let mut sys = LinSys::new(n * n);
    for i in 0..n {
        for j in 0..n {
            let k = idx(i, j);
            for (di, dj) in [(0i64, 1i64), (0, -1), (1, 0), (-1, 0)] {
                let (ni, nj) = (i as i64 + di, j as i64 + dj);
                if ni < 0 || nj < 0 || ni >= n as i64 || nj >= n as i64 {
                    continue;
                }
                sys.diag[k] += g;
                sys.offdiag[k].push((idx(ni as usize, nj as usize), g));
            }
            // the left column also touches the supply: an extra conductance to a fixed node,
            // which lands on the diagonal and in the rhs rather than in offdiag.
            if j == 0 {
                sys.diag[k] += g;
                sys.rhs[k] += g * vdd;
            }
        }
    }
    let corner = idx(n - 1, n - 1);
    sys.rhs[corner] -= load;
    (sys, corner)
}

/// The regression that matters: a mesh of real-PDN size must solve, and quickly.
///
/// 80×80 is 6 400 nodes, the order of the 5 308-node grid a routed sky130 block extracts to —
/// the one where Gauss-Seidel stopped at 1.1e-7 against a 1e-8 tolerance after 50 000 sweeps
/// and the engine returned an error instead of a result.
///
/// The **iteration cap is the assertion**. 500 is far below what a stationary method needs on
/// a mesh this wide (its convergence rate is set by the mesh diameter), so reverting to one
/// fails here rather than merely getting slower.
#[test]
fn a_real_sized_mesh_converges_within_a_modest_iteration_cap() {
    let (sys, corner) = mesh(80, 1.0, 1.8, 1.0);
    let x = sys
        .solve(500, 1e-9)
        .expect("a 6400-node mesh must solve; this is the size a real PDN extracts to");

    // Sanity on the answer itself: every node sits between the supply and the loaded corner,
    // and the corner is the minimum because it is the only place current leaves.
    let vmin = x.iter().cloned().fold(f64::INFINITY, f64::min);
    let vmax = x.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    assert!(vmax <= 1.8 + 1e-9, "no node exceeds the supply: {vmax}");
    assert!(
        (x[corner] - vmin).abs() < 1e-12,
        "the loaded corner is the worst node"
    );
    assert!(vmin < 1.8, "a load must produce some droop");
}

/// An oracle-free check that the answer solves the system it was given.
///
/// Convergence flags say the method stopped; they do not say the result is right. Recomputing
/// `A·x − b` from the stored form is independent of however the solve reached `x`, so it
/// catches a sign error or a preconditioner applied in the wrong place — neither of which
/// disturbs the iteration count.
#[test]
fn the_solution_actually_satisfies_the_system() {
    let (sys, _) = mesh(40, 2.5, 1.8, 0.3);
    let x = sys.solve(500, 1e-10).unwrap();

    let mut worst = 0.0f64;
    for k in 0..sys.n {
        let mut ax = sys.diag[k] * x[k];
        for &(j, g) in &sys.offdiag[k] {
            ax -= g * x[j];
        }
        worst = worst.max((ax - sys.rhs[k]).abs() / sys.diag[k]);
    }
    assert!(
        worst < 1e-9,
        "A*x - b implies a per-node voltage error of {worst}"
    );
}
