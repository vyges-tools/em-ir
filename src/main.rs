//! vyges-em-ir CLI.
//!
//!   vyges-em-ir run   JOB [-o OUT] [--json] [--fail-on-violation]   analyze -> report
//!   vyges-em-ir check JOB                                          validate the job
//!   vyges-em-ir demo  [-o OUT] [--json]                            built-in PDN
//!
//! Common flags: -h/--help, -V/--version, -q/--quiet, -v/--verbose.
//! Exit codes: 0 ok · 1 runtime/solver error · 2 usage/validation · 3 IR/EM
//! violation (only with --fail-on-violation).

use std::process::exit;

use vyges_em_ir::emir::EmIrReport;
use vyges_em_ir::engine;
use vyges_em_ir::job::EmIrJob;

const USAGE: &str = "\
vyges-em-ir — EM / IR-drop power-integrity sign-off (PDN -> report)

usage:
  vyges-em-ir run   JOB [-o OUT] [--json] [--fail-on-violation]
  vyges-em-ir check JOB
  vyges-em-ir demo  [-o OUT] [--json]

flags:
  -o FILE               write output to FILE (default: stdout)
  --json                machine-readable JSON instead of the text report
  --fail-on-violation   exit 3 if IR drop exceeds the limit or any EM segment fails
  -q, --quiet           suppress non-essential output
  -v, --verbose         extra detail on stderr
  -h, --help            show this help
  -V, --version         show version
  --bug-report     file a bug (central: vyges/community)
  --feature-request request a feature (central)
  --sponsor        sponsor Vyges (github.com/sponsors/vyges-ip)
  --star           star this tool on GitHub ⭐
";

const BUG_URL: &str =
    "https://github.com/vyges/community/issues/new?template=bug_report_template.yaml";
const FEATURE_URL: &str = "https://github.com/vyges/community/issues/new?labels=enhancement";
const SPONSOR_URL: &str = "https://github.com/sponsors/vyges-ip";
const STAR_URL: &str = "https://github.com/vyges-tools/em-ir";

/// Print a labelled URL; if stdout is a terminal, also try to open it in a browser.
/// In headless / agent contexts (not a TTY) it just prints the URL.
fn link(label: &str, url: &str) {
    use std::io::IsTerminal;
    println!("{label}:\n  {url}");
    if std::io::stdout().is_terminal() {
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        let _ = std::process::Command::new(opener).arg(url).status();
    }
}

#[derive(Default)]
struct Cli {
    positionals: Vec<String>,
    out: Option<String>,
    json: bool,
    quiet: bool,
    verbose: bool,
    fail_on_violation: bool,
    help: bool,
    version: bool,
    bug_report: bool,
    feature_request: bool,
    sponsor: bool,
    star: bool,
}

fn parse_cli(args: &[String]) -> Cli {
    let mut c = Cli::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                c.out = args.get(i + 1).cloned();
                i += 1;
            }
            "--json" => c.json = true,
            "--fail-on-violation" => c.fail_on_violation = true,
            "-q" | "--quiet" => c.quiet = true,
            "-v" | "--verbose" => c.verbose = true,
            "-h" | "--help" => c.help = true,
            "-V" | "--version" => c.version = true,
            "--bug-report" => c.bug_report = true,
            "--feature-request" => c.feature_request = true,
            "--sponsor" => c.sponsor = true,
            "--star" => c.star = true,
            other => c.positionals.push(other.to_string()),
        }
        i += 1;
    }
    c
}

fn write_out(text: &str, cli: &Cli) {
    match &cli.out {
        Some(path) => match std::fs::write(path, text) {
            Ok(_) => {
                if !cli.quiet {
                    println!("wrote {path}");
                }
            }
            Err(e) => {
                eprintln!("error: {path}: {e}");
                exit(1);
            }
        },
        None => print!("{text}"),
    }
}

fn emit(job: &EmIrJob, rep: &EmIrReport, cli: &Cli) -> ! {
    let text = if cli.json {
        engine::report_json(job, rep)
    } else {
        engine::render_report(job, rep)
    };
    write_out(&text, cli);
    if cli.fail_on_violation && !engine::passes(job, rep) {
        if !cli.quiet {
            eprintln!("power-integrity VIOLATED (IR drop or EM over limit)");
        }
        exit(3);
    }
    exit(0);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = parse_cli(&args);

    if cli.bug_report {
        return link("Report a bug (central — vyges/community)", BUG_URL);
    }
    if cli.feature_request {
        return link("Request a feature (central — vyges/community)", FEATURE_URL);
    }
    if cli.sponsor {
        return link("Sponsor Vyges", SPONSOR_URL);
    }
    if cli.star {
        return link("Star vyges-em-ir on GitHub ⭐", STAR_URL);
    }
    if cli.version {
        println!("vyges-em-ir {} ({})", vyges_em_ir::VERSION, env!("VYGES_GIT_SHA"));
        println!("{}", vyges_em_ir::COPYRIGHT);
        return;
    }
    let cmd = cli.positionals.first().cloned().unwrap_or_default();
    if cli.help || cmd.is_empty() {
        print!("{USAGE}");
        exit(if cmd.is_empty() && !cli.help { 2 } else { 0 });
    }

    match cmd.as_str() {
        "demo" => {
            let (job, rep) = engine::demo();
            emit(&job, &rep, &cli);
        }
        "check" => {
            let Some(path) = cli.positionals.get(1) else {
                eprintln!("usage: vyges-em-ir check JOB");
                exit(2);
            };
            match EmIrJob::load(path) {
                Ok(j) => println!(
                    "OK  design={} pdn={} ir_limit_pct={}",
                    j.design, j.pdn, j.ir_limit_pct
                ),
                Err(e) => {
                    eprintln!("error: {e}");
                    exit(2);
                }
            }
        }
        "run" => {
            let Some(path) = cli.positionals.get(1) else {
                eprintln!("usage: vyges-em-ir run JOB [-o OUT]");
                exit(2);
            };
            let job = match EmIrJob::load(path) {
                Ok(j) => j,
                Err(e) => {
                    eprintln!("error: {e}");
                    exit(2);
                }
            };
            if cli.verbose {
                eprintln!("solving PDN {}", job.pdn);
            }
            match engine::analyze_job(&job) {
                Ok(rep) => emit(&job, &rep, &cli),
                Err(e) => {
                    eprintln!("error: {e}");
                    exit(1);
                }
            }
        }
        other => {
            eprintln!("vyges-em-ir: unknown command {other:?}\n");
            print!("{USAGE}");
            exit(2);
        }
    }
}
