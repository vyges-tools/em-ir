//! Minimal DEF reader for PDN extraction — the special-net power-grid geometry.
//!
//! Parses `UNITS DISTANCE MICRONS <dbu>` and the `SPECIALNETS` section: each net's
//! `+ ROUTED`/`NEW <layer> <width>` wires as a polyline of points (in DB units),
//! plus the via placements along them. Only the **power** net is kept (the one with
//! `+ USE POWER`, else a `VPWR`/`VDD`/`VCCD`/`VCC` name, else the first net) — supply
//! IR drop is the v1 analysis. Co-ordinate shorthand `( * y )` / `( x * )` (reuse the
//! previous coordinate) is handled.
//!
//! Pure std — fully unit-tested offline. Routing/RECT special-wire shapes and the
//! full via-stack layer resolution are out of scope for v1 (vias bridge whatever
//! layers have a node at the via point).

#[derive(Debug, Clone)]
pub struct Seg {
    pub layer: String,
    pub width_dbu: f64,
    pub x1: i64,
    pub y1: i64,
    pub x2: i64,
    pub y2: i64,
}

#[derive(Debug, Clone, Default)]
pub struct NetGeom {
    pub name: String,
    pub use_power: bool,
    pub segs: Vec<Seg>,
    pub vias: Vec<(i64, i64)>,
}

#[derive(Debug, Clone)]
pub struct Def {
    pub dbu: f64, // database units per micron
    pub nets: Vec<NetGeom>,
}

#[derive(Debug)]
pub struct DefError(pub String);
impl std::fmt::Display for DefError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "def error: {}", self.0)
    }
}
impl std::error::Error for DefError {}

const POWER_NAMES: &[&str] = &["VPWR", "VDD", "VCCD", "VCC", "VDDP"];

impl Def {
    /// The power net to analyze: `USE POWER`, else a known power name, else the first.
    pub fn power_net(&self) -> Option<&NetGeom> {
        self.nets
            .iter()
            .find(|n| n.use_power)
            .or_else(|| self.nets.iter().find(|n| POWER_NAMES.contains(&n.name.as_str())))
            .or_else(|| self.nets.first())
    }

    pub fn parse(text: &str) -> Result<Def, DefError> {
        let toks: Vec<&str> = text.split_whitespace().collect();
        let mut dbu = 1000.0;
        // UNITS DISTANCE MICRONS <dbu> ;
        for w in toks.windows(4) {
            if w[0] == "UNITS" && w[1] == "DISTANCE" && w[2] == "MICRONS" {
                dbu = w[3].trim_end_matches(';').parse().unwrap_or(1000.0);
            }
        }
        // slice the SPECIALNETS … END SPECIALNETS span
        let start = toks.iter().position(|&t| t == "SPECIALNETS");
        let Some(s) = start else {
            return Err(DefError("no SPECIALNETS section".into()));
        };
        let end = (s..toks.len())
            .find(|&i| toks[i] == "END" && toks.get(i + 1) == Some(&"SPECIALNETS"))
            .unwrap_or(toks.len());
        let body = &toks[s + 1..end];

        let nets = parse_specialnets(body);
        if nets.is_empty() {
            return Err(DefError("no special nets parsed".into()));
        }
        Ok(Def { dbu, nets })
    }

    pub fn load(path: &str) -> Result<Def, DefError> {
        let text = std::fs::read_to_string(path).map_err(|e| DefError(format!("{path}: {e}")))?;
        Def::parse(&text)
    }
}

/// Walk the SPECIALNETS token stream into per-net geometry.
fn parse_specialnets(body: &[&str]) -> Vec<NetGeom> {
    let mut nets: Vec<NetGeom> = Vec::new();
    let mut cur: Option<NetGeom> = None;
    let mut layer = String::new();
    let mut width = 0.0f64;
    let mut last: Option<(i64, i64)> = None; // last point on the current wire
    let mut i = 0;
    while i < body.len() {
        let t = body[i];
        match t {
            "-" => {
                // close the previous net, open a new one
                if let Some(n) = cur.take() {
                    nets.push(n);
                }
                let name = body.get(i + 1).copied().unwrap_or("").to_string();
                cur = Some(NetGeom { name, ..Default::default() });
                last = None;
                i += 2;
            }
            ";" => {
                last = None; // end of this net's routing statement
                i += 1;
            }
            "USE" => {
                if body.get(i + 1) == Some(&"POWER") {
                    if let Some(n) = cur.as_mut() {
                        n.use_power = true;
                    }
                }
                i += 2;
            }
            "ROUTED" | "NEW" => {
                layer = body.get(i + 1).copied().unwrap_or("").to_string();
                width = body.get(i + 2).and_then(|w| w.parse().ok()).unwrap_or(0.0);
                last = None;
                i += 3;
            }
            "(" => {
                // a point: ( x y [ext] )  — x/y may be `*` (reuse previous). Non-coordinate
                // paren groups (e.g. `( PIN VGND )`, `( * VNB )` connection refs) are skipped.
                let xr = body.get(i + 1).copied().unwrap_or("0");
                let yr = body.get(i + 2).copied().unwrap_or("0");
                let prev = last.unwrap_or((0, 0));
                let px_ok = xr == "*" || xr.parse::<i64>().is_ok();
                let py_ok = yr == "*" || yr.parse::<i64>().is_ok();
                // advance past the closing ')'
                let mut j = i + 1;
                while j < body.len() && body[j] != ")" {
                    j += 1;
                }
                let next_i = j + 1;
                if !px_ok || !py_ok {
                    i = next_i; // not a coordinate (a PIN/connection ref) — skip it
                    continue;
                }
                let x = if xr == "*" { prev.0 } else { xr.parse().unwrap_or(0) };
                let y = if yr == "*" { prev.1 } else { yr.parse().unwrap_or(0) };
                i = next_i;
                if let (Some(n), Some((px, py))) = (cur.as_mut(), last) {
                    if px != x || py != y {
                        n.segs.push(Seg {
                            layer: layer.clone(),
                            width_dbu: width,
                            x1: px,
                            y1: py,
                            x2: x,
                            y2: y,
                        });
                    }
                }
                last = Some((x, y));
            }
            "+" => i += 1, // qualifier marker; the keyword that follows is handled above
            other => {
                // a bare identifier in the point stream = a via at the last point;
                // skip SHAPE/STYLE/etc. qualifier args (they follow a handled keyword).
                if other.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false) {
                    if let (Some(n), Some(p)) = (cur.as_mut(), last) {
                        // treat as a via only if it looks like a via name, not a keyword
                        if !is_qualifier(other) {
                            n.vias.push(p);
                        }
                    }
                }
                i += 1;
            }
        }
    }
    if let Some(n) = cur.take() {
        nets.push(n);
    }
    nets
}

fn is_qualifier(t: &str) -> bool {
    matches!(
        t,
        "SHAPE" | "STRIPE" | "FOLLOWPIN" | "STYLE" | "FIXED" | "COVER" | "POWER" | "GROUND"
            | "RECT" | "PIN" | "MASK" | "RING" | "BLOCKWIRE" | "PADRING" | "BLOCKAGEWIRE"
    )
}
