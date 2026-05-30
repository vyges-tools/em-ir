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
    assert!(j.trim_end().ends_with('}'));
}
