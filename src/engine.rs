//! EM/IR engine wiring: job → PDN → analyze → report text/JSON.
//!
//! Files in / report out; no subprocess (v0 is fully self-contained). OpenROAD
//! PDNSim is the correlation baseline this engine is checked against, not a
//! runtime dependency.

use crate::emir::{self, EmIrError, EmIrReport};
use crate::job::EmIrJob;
use crate::pdn::PdnSpec;

const DEMO_PDN: &str = "\
vdd 1.8
pad p
res p  n1 0.1 met5
res n1 n2 0.2 met4
load n1 0.5
load n2 0.5
emlimit met5 0.8
emlimit met4 0.3
";

pub fn analyze_job(job: &EmIrJob) -> Result<EmIrReport, EmIrError> {
    let spec = PdnSpec::load(&job.resolve(&job.pdn)).map_err(|e| EmIrError::Parse(e.to_string()))?;
    emir::analyze(&spec)
}

/// Built-in PDN analyzed offline (for `demo`).
pub fn demo() -> (EmIrJob, EmIrReport) {
    let job = EmIrJob {
        design: "demo".into(),
        pdn: "(builtin)".into(),
        ir_limit_pct: 5.0,
        base_dir: String::new(),
    };
    let spec = PdnSpec::parse(DEMO_PDN).expect("builtin pdn parses");
    let rep = emir::analyze(&spec).expect("builtin pdn analyzes");
    (job, rep)
}

/// Whether the report passes the job's IR-drop and EM gates. When a transient
/// analysis ran, the **dynamic** droop is the binding IR check (it's the deeper one).
pub fn passes(job: &EmIrJob, rep: &EmIrReport) -> bool {
    let ir_pct = match &rep.dynamic {
        Some(d) => d.drop_pct,
        None => rep.worst_ir.as_ref().map(|w| w.drop_pct).unwrap_or(0.0),
    };
    ir_pct <= job.ir_limit_pct && rep.em_violations.is_empty()
}

pub fn render_report(job: &EmIrJob, rep: &EmIrReport) -> String {
    let mut s = String::new();
    s.push_str(&format!("EM/IR report — design {}\n", job.design));
    s.push_str(&format!(
        "  vdd {:.3} V   nodes {}   ir_limit {:.1}%\n",
        rep.vdd, rep.nodes, job.ir_limit_pct
    ));
    match &rep.worst_ir {
        Some(w) => {
            let verdict = if w.drop_pct <= job.ir_limit_pct { "MET" } else { "VIOLATED" };
            s.push_str(&format!(
                "  worst IR drop: {:.4} V ({:.2}%) at {}   [Vmin {:.4} V]   [{}]\n",
                w.drop, w.drop_pct, w.node, w.voltage, verdict
            ));
        }
        None => s.push_str("  worst IR drop: (no free nodes)\n"),
    }
    if let Some(d) = &rep.dynamic {
        let verdict = if d.drop_pct <= job.ir_limit_pct { "MET" } else { "VIOLATED" };
        s.push_str(&format!(
            "  worst DYNAMIC droop: {:.4} V ({:.2}%) at {} @ {:.3} ns   [Vmin {:.4} V]   [{}]\n",
            d.drop, d.drop_pct, d.node, d.time_ns, d.voltage, verdict
        ));
    }
    if rep.em_checked == 0 {
        s.push_str("  EM: no segments had a layer limit (set `emlimit <layer> <A>`)\n");
    } else if rep.em_violations.is_empty() {
        s.push_str(&format!(
            "  EM: 0 / {} segments over limit (worst {:.2}x)   [MET]\n",
            rep.em_checked, rep.em_worst_ratio
        ));
    } else {
        s.push_str(&format!(
            "  EM: {} / {} segments over limit (worst {:.2}x)   [VIOLATED]\n",
            rep.em_violations.len(),
            rep.em_checked,
            rep.em_worst_ratio
        ));
        s.push_str(&format!("    {:>6}  {:>9}  {:>7}  layer  segment\n", "ratio", "current", "limit"));
        let mut v = rep.em_violations.clone();
        v.sort_by(|a, b| b.ratio.partial_cmp(&a.ratio).unwrap_or(std::cmp::Ordering::Equal));
        for e in &v {
            s.push_str(&format!(
                "    {:5.2}x  {:8.4}A  {:7.3}  {:5}  {}-{}\n",
                e.ratio, e.current, e.limit, e.layer, e.a, e.b
            ));
        }
    }
    s
}

pub fn report_json(job: &EmIrJob, rep: &EmIrReport) -> String {
    let mut s = String::new();
    s.push_str(&format!("{{\"design\":{:?},\"vdd\":{:.6},\"nodes\":{},", job.design, rep.vdd, rep.nodes));
    match &rep.worst_ir {
        Some(w) => s.push_str(&format!(
            "\"worst_ir\":{{\"node\":{:?},\"voltage\":{:.6},\"drop\":{:.6},\"drop_pct\":{:.4}}},",
            w.node, w.voltage, w.drop, w.drop_pct
        )),
        None => s.push_str("\"worst_ir\":null,"),
    }
    match &rep.dynamic {
        Some(d) => s.push_str(&format!(
            "\"dynamic\":{{\"node\":{:?},\"voltage\":{:.6},\"drop\":{:.6},\"drop_pct\":{:.4},\"time_ns\":{:.4}}},",
            d.node, d.voltage, d.drop, d.drop_pct, d.time_ns
        )),
        None => s.push_str("\"dynamic\":null,"),
    }
    // IR met: the dynamic droop binds when present, else the static drop.
    let ir_pct = rep
        .dynamic
        .as_ref()
        .map(|d| d.drop_pct)
        .or_else(|| rep.worst_ir.as_ref().map(|w| w.drop_pct))
        .unwrap_or(0.0);
    s.push_str(&format!(
        "\"ir_met\":{},\"em_checked\":{},\"em_worst_ratio\":{:.4},",
        ir_pct <= job.ir_limit_pct,
        rep.em_checked,
        rep.em_worst_ratio
    ));
    s.push_str("\"em_violations\":[");
    for (i, e) in rep.em_violations.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"a\":{:?},\"b\":{:?},\"layer\":{:?},\"current\":{:.6},\"limit\":{:.6},\"ratio\":{:.4}}}",
            e.a, e.b, e.layer, e.current, e.limit, e.ratio
        ));
    }
    s.push_str("]}\n");
    s
}
