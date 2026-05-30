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

pub mod job;
pub mod pdn;
pub mod solver;
pub mod emir;
pub mod engine;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
