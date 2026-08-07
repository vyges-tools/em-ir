//! Dump the extracted PDN resistor network as per-layer statistics.
//!
//! The instrument the PDNSim network correlation needs: PDNSim's `write_pg_spice`
//! emits its own network with values, and this emits ours from the same DEF+LEF, in
//! the same shape, so the two can be compared **before** any current or voltage is
//! involved. Node sets differ by construction (PDNSim resamples on a minimum node
//! pitch; we place a node per polyline point), so the comparable quantity is the
//! electrical aggregate per layer, not the node count.
//!
//!   cargo run --example dump_pdn -- <job.emir>
//!
//! Emits JSON on stdout: per-layer resistor count, total and mean resistance, total
//! conductance, plus node and source counts.

use std::collections::BTreeMap;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(job_path) = args.next() else {
        eprintln!("usage: dump_pdn <job.emir>");
        std::process::exit(2);
    };

    let job = match vyges_em_ir::job::EmIrJob::load(&job_path) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("job: {e}");
            std::process::exit(2);
        }
    };
    let def_text = std::fs::read_to_string(job.resolve(&job.def)).expect("read def");
    let def = vyges_em_ir::def::Def::parse(&def_text).expect("parse def");
    let lef_text = std::fs::read_to_string(job.resolve(&job.lef)).expect("read lef");
    let lef = vyges_em_ir::lef::TechLef::parse(&lef_text).expect("parse lef");
    let spec = vyges_em_ir::extract::extract(&def, &lef, &job).expect("extract");

    #[derive(Default)]
    struct LayerStat {
        n: usize,
        total_r: f64,
        total_g: f64,
    }
    let mut per_layer: BTreeMap<String, LayerStat> = BTreeMap::new();
    let mut nodes: std::collections::BTreeSet<&str> = Default::default();
    for r in &spec.resistors {
        let key = r.layer.clone().unwrap_or_else(|| "<none>".into());
        let e = per_layer.entry(key).or_default();
        e.n += 1;
        e.total_r += r.r;
        e.total_g += 1.0 / r.r;
        nodes.insert(&r.a);
        nodes.insert(&r.b);
    }

    println!("{{");
    println!("  \"resistors\": {},", spec.resistors.len());
    println!("  \"nodes\": {},", nodes.len());
    println!("  \"sources\": {},", spec.pads.len());
    println!("  \"loads\": {},", spec.loads.len());
    println!("  \"per_layer\": {{");
    let n = per_layer.len();
    for (i, (layer, s)) in per_layer.iter().enumerate() {
        let comma = if i + 1 == n { "" } else { "," };
        println!(
            "    \"{}\": {{ \"resistors\": {}, \"total_r\": {:.6e}, \"mean_r\": {:.6e}, \"total_g\": {:.6e} }}{}",
            layer,
            s.n,
            s.total_r,
            s.total_r / s.n as f64,
            s.total_g,
            comma
        );
    }
    println!("  }}");
    println!("}}");
}
