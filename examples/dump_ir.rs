//! Solve the extracted PDN and dump every node's voltage as CSV.
//!
//! The per-record half of the PDNSim correlation. PDNSim's `analyze_power_grid -voltage_file`
//! writes one row per instance terminal (instance, terminal, layer, x, y, voltage); this writes
//! one row per grid node (layer, x, y, voltage, drop). The two are joined by position, because
//! the node sets are not the same by construction — PDNSim resamples on a minimum node pitch and
//! anchors each instance to the nodes its terminals touch, while this engine places a node per
//! polyline point and lands an instance's current on the nearest rail node.
//!
//!   cargo run --release --example dump_ir -- <job.emir>
//!
//! Coordinates are DEF database units, as the node names carry them.

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(job_path) = args.next() else {
        eprintln!("usage: dump_ir <job.emir>");
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

    // Solve the same way `run` does, but keep every node instead of the worst one.
    let sys_nodes = vyges_em_ir::emir::node_voltages(&spec).expect("solve");

    println!("layer,x,y,voltage,drop");
    for (name, v) in &sys_nodes {
        // node names are `<layer>_<x>_<y>` — split from the right so layer names keep any
        // underscore of their own.
        let mut it = name.rsplitn(3, '_');
        let (y, x, layer) = (
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
        );
        println!("{layer},{x},{y},{v:.9},{:.9}", spec.vdd - v);
    }
}
