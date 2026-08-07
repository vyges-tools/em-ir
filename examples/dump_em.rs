//! Dump every PDN segment's current as CSV, in PDNSim's `-em_outfile` column order.
//!
//! The per-record half of the EM correlation. PDNSim reports per-segment current but applies
//! no limit and issues no verdict, so its EM output is an oracle for our **numerator** only —
//! which is the useful half, because it isolates the current computation from the LEF limit
//! lookup that only this engine does.
//!
//!   cargo run --release --example dump_em -- <job.emir>
//!
//! Columns match `writeEMFile`: both endpoints' layer and position, then the current.
//! Coordinates are emitted in microns, as PDNSim does, so the two files join directly.

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(job_path) = args.next() else {
        eprintln!("usage: dump_em <job.emir>");
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
    let dbu = def.dbu;

    // node -> volts, with pads at the supply. Free-node voltages come from the same solve
    // the report uses; pads are held and never appear among them.
    let mut v: std::collections::HashMap<String, f64> =
        vyges_em_ir::emir::node_voltages(&spec)
            .expect("solve")
            .into_iter()
            .collect();
    for (n, pv) in &spec.pads {
        v.insert(n.clone(), *pv);
    }

    // `<layer>_<x>_<y>` -> (layer, x µm, y µm); split from the right so a layer name may
    // contain an underscore of its own.
    let split = |name: &str| -> (String, f64, f64) {
        let mut it = name.rsplitn(3, '_');
        let y: f64 = it.next().unwrap_or("0").parse().unwrap_or(0.0);
        let x: f64 = it.next().unwrap_or("0").parse().unwrap_or(0.0);
        (it.next().unwrap_or("").to_string(), x / dbu, y / dbu)
    };

    println!(
        "Node0 Layer,Node0 X location,Node0 Y location,Node1 Layer,Node1 X location,Node1 Y location,Current"
    );
    for r in &spec.resistors {
        let (Some(&va), Some(&vb)) = (v.get(&r.a), v.get(&r.b)) else {
            continue;
        };
        let i = (va - vb).abs() / r.r;
        let (la, ax, ay) = split(&r.a);
        let (lb, bx, by) = split(&r.b);
        println!("{la},{ax:.4},{ay:.4},{lb},{bx:.4},{by:.4},{i:.3e}");
    }
}
