//! vyges-em-ir — EM / IR-drop power-integrity sign-off.
//!
//! The power delivery network (PDN) is a resistive mesh of supply straps and
//! vias. Real current draw through that mesh makes the on-chip supply sag below
//! nominal (**IR drop**) and pushes current density in wires toward the
//! electromigration limit (**EM**). This engine solves the PDN as a resistor
//! network: given the mesh, the supply pads, and per-node current loads, it
//! solves for every node voltage, reports the worst IR drop, and flags segments
//! over their EM current limit.
//!
//! Boundaries (per the Vyges flow architecture): inputs and outputs are files
//! (a PDN resistor network in, a power-integrity report out). The whole v0 is
//! pure std and unit-tested offline — no subprocess. OpenROAD's PDNSim is the
//! correlation baseline, not a runtime dependency.
//!
//! v0 scope: static (DC) IR drop via a Gauss-Seidel solve of the conductance
//! system, plus per-layer EM current-limit checks. Geometry extraction from
//! DEF/LEF, dynamic/transient IR, and electrothermal coupling (the BCD/power
//! axis) build on the same network model; the engine reserves the
//! `EmIrError::ElectrothermalNotModeled` hook.

// DEF reader now comes from the shared vyges-loom foundation. loom's Def is a
// superset; the power view em-ir uses (power_net()/dbu/comps, NetGeom/Seg/Comp)
// is unchanged.
pub use vyges_loom::def;
pub mod emir;
pub mod engine;
pub mod extract;
pub mod job;
/// tech-LEF reader now comes from the shared vyges-loom foundation. loom's `Lef`
/// is a superset (PDN/EM fields + extraction's width/thickness); re-exported under
/// em-ir's historical names so the rest of the engine is unchanged.
pub mod lef {
    pub use vyges_loom::lef::{Layer as LayerR, Lef as TechLef, LefError};
}
pub mod pdn;
pub mod solver;
/// EM geometry sidecar reader, re-exported from loom — the per-segment layer/width
/// the extracted-SPEF EM sign-off screens.
pub use vyges_loom::emgeom;
/// Extracted-SPEF electromigration sign-off (signal-net current density vs LEF
/// `DCCURRENTDENSITY`), a companion to the DEF+LEF PDN EM path.
pub mod emsignoff;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COPYRIGHT: &str = "© 2026 Vyges. All Rights Reserved.  https://vyges.com";
