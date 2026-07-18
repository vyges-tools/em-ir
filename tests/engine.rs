//! End-to-end: the example PDN analyzes offline (v0 is pure-std, no subprocess).

use vyges_em_ir::engine::{analyze_job, passes, report_json};
use vyges_em_ir::job::EmIrJob;

#[test]
fn example_block_analyzes_clean() {
    let job_path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/block/block.emir");
    let job = EmIrJob::load(job_path).unwrap();
    let rep = analyze_job(&job).unwrap();

    assert!(rep.nodes >= 3); // a, b, c free nodes
    let w = rep.worst_ir.as_ref().unwrap();
    assert!(w.drop_pct < 5.0, "drop_pct={}", w.drop_pct); // light load -> meets
    assert!(passes(&job, &rep));

    let j = report_json(&job, &rep);
    assert!(j.contains("\"design\":\"block\""));
    assert!(j.contains("\"ir_met\":true"));
    assert!(j.contains("\"pi_met\":true"));
    assert!(j.trim_end().ends_with('}'));
}

/// `pi_met` is the whole power-integrity verdict; `ir_met` is only half of it.
/// The demo carries both an over-limit IR drop and EM violations, so a consumer
/// reading `ir_met` alone would miss the EM half on a design that met IR.
#[test]
fn pi_met_covers_em_not_just_ir() {
    let (job, rep) = vyges_em_ir::engine::demo();
    assert!(
        !rep.em_violations.is_empty(),
        "demo should carry EM violations"
    );
    assert!(!passes(&job, &rep));

    let j = report_json(&job, &rep);
    assert!(j.contains("\"pi_met\":false"));

    // Now relax the IR limit so IR alone would report met — `pi_met` must still
    // be false, because the EM violations are untouched.
    let mut relaxed = job.clone();
    relaxed.ir_limit_pct = 100.0;
    let j = report_json(&relaxed, &rep);
    assert!(j.contains("\"ir_met\":true"), "IR now within limit: {j}");
    assert!(
        j.contains("\"pi_met\":false"),
        "EM violations must still fail the verdict: {j}"
    );
}
