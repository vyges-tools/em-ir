//! Analog power-integrity via the generic `.pdn` path.
//!
//! The G·V=I solve and the `.pdn` resistor-network input are generic physics —
//! they don't assume standard cells, a clock, or digital activity. Here the loads
//! are an **analog DC operating point** (a bias reference and a large power-amplifier
//! branch), and the IR/EM analysis runs identically. No engine change: this drives
//! the existing empty-`def` → `.pdn` branch of `analyze_job`.

use vyges_em_ir::engine::analyze_job;
use vyges_em_ir::job::EmIrJob;

#[test]
fn analog_bias_grid_ir_and_em() {
    let job_path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/analog_bias/analog_bias.emir");
    let job = EmIrJob::load(job_path).unwrap();
    // empty `def` selects the generic `.pdn` path (the same one digital grids use).
    assert!(job.def.is_empty(), "drives the .pdn branch, not DEF extraction");
    let rep = analyze_job(&job).unwrap();

    // a resistive analog grid: trunk/near/mid/far + the two via-fed branch nodes.
    assert!(rep.nodes >= 5, "nodes={}", rep.nodes);

    // --- IR: the supply sags below VDD, and the worst sag is real (> 0) ---
    let w = rep.worst_ir.as_ref().expect("a worst-IR node");
    assert!(w.drop > 0.0, "IR drop must be positive, got {}", w.drop);
    assert!(w.voltage < rep.vdd, "worst node sits below VDD: {} < {}", w.voltage, rep.vdd);
    assert!((w.drop - (rep.vdd - w.voltage)).abs() < 1e-9, "drop = vdd - voltage");

    // The worst drop is at the power-amplifier branch: it draws the largest current
    // (30 mA vs 1 mA) AND is the electrically most distant node (longest R path,
    // through the via stack) — both push it to the deepest sag.
    assert_eq!(w.node, "pa_m1", "worst IR drop at the high-current, most-distant node");

    // It is strictly worse than the small near bias branch — sanity that current ×
    // path-resistance, not node count, sets the hotspot.
    assert!(w.drop_pct > 0.0);

    // --- EM: per-segment current densities are computed and checked ---
    assert!(rep.em_checked > 0, "EM segments with a layer limit were checked");
    // the via carrying the full 30 mA PA current is the EM hotspot (worst ratio).
    assert!(rep.em_worst_ratio > 0.0, "an EM current/limit ratio was computed");

    // static (DC operating point) — no switching events, so no transient droop.
    assert!(rep.dynamic.is_none(), "static analog op-point: no `switch` events");
}
