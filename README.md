# vyges-em-ir

**EM / IR-drop power-integrity sign-off**: a power-grid resistor network in, a
power-integrity report out.

> **Vyges open EDA tools.** Commercial-grade silicon sign-off capability, built
> on open standards and plain file formats — and meant to be accessible to
> everyone, not only teams who can license a six-figure tool. `vyges-em-ir`
> opens up power-integrity sign-off.

## Why this exists

Logic only works if the power gets there. The power-distribution network (PDN) —
the mesh of supply straps and vias — has resistance, so real current draw makes
the on-chip supply **sag below nominal** (IR drop), which slows or breaks timing,
and pushes current density in the wires toward the **electromigration** limit,
which shortens the chip's life. Power sign-off proves both are within budget.

## How this is solved today

In production, power sign-off is Cadence **Voltus** or Ansys **RedHawk-SC** —
static and dynamic IR, EM rules, electrothermal — gated behind major licenses.
The open baseline is **PDNSim** (in OpenROAD), which does static IR and basic EM.
`vyges-em-ir` is an open engine in that space, behind a plain resistor-network
file format, correlated against PDNSim as its baseline.

## The problem it solves

Given a **PDN resistor network** — supply pads at a fixed voltage, resistive
strap/via segments, and per-node current loads — it solves the conductance
system for **every node voltage**, then reports:

- the **worst IR drop** (supply sag vs nominal, in volts and % of vdd), and
- every **EM** segment whose current `|Δv|/R` exceeds its per-layer limit.

The solve is Gauss-Seidel over the reduced free-node system (the PDN is
diagonally dominant, so it converges).

## When & how to use it in your flow

```text
  layout (PDN) ──► PDN resistor network ──┐
  per-instance power ──► node currents ───┤
                                          ▼
                            ┌───────────────────────────┐
                            │        vyges-em-ir         │
                            └───────────────────────────┘
                                          │
                                          ▼
                   worst IR drop + EM violations ──► within budget? sign off :
                                                     widen straps / add pads / vias
```

Run it **after place-and-route and PDN generation** (you have the supply grid)
and **after a power estimate** (so you know the per-node current draw), and
**before tape-out**. What it gives you is the **answer to "does my power grid
hold up?"** — the worst IR-drop node and the over-limit EM segments tell you
exactly where to widen straps, add vias, or add pad connections. In the open
RTL→GDS flow it occupies the slot where PDNSim runs.

## Use it

```sh
cargo build --release            # std-only, no external deps

vyges-em-ir run  block.emir -o block.rpt          # analyze -> report
vyges-em-ir run  block.emir --json                # machine-readable IR/EM
vyges-em-ir run  block.emir --fail-on-violation   # exit 3 if IR/EM over budget (CI gate)
vyges-em-ir check block.emir                       # validate the job + inputs
vyges-em-ir demo                                   # analyze a built-in PDN
# common flags: -o FILE · --json · -q/--quiet · -v/--verbose · -h/--help · -V/--version
```

A job (`*.emir`) points at a PDN and sets the IR budget:

```text
design:       block
pdn:          block.pdn
ir_limit_pct: 5.0        # fail if worst IR drop exceeds 5% of vdd
```

A PDN (`*.pdn`) is a small resistor network:

```text
vdd 1.8
pad p1                  # supply pad, tied to vdd
res p1 a 0.05 met5      # resistor: nodeA nodeB ohms [layer]
via a  m1 2.0           # a via resistance
load c 0.010            # current drawn out of a node (amps)
emlimit met5 0.50       # per-layer EM current limit (amps/segment)
```

A complete, runnable example is in [`examples/block/`](examples/block/);
`vyges-em-ir run examples/block/block.emir` reports IR drop + EM on a small mesh.

## Open core, certified fab plugins

`vyges-em-ir` is open and contains **no foundry-confidential data**. It runs out
of the box on any PDN network you describe. What is fab-specific — the per-layer
**EM current-density limits** and the **electrothermal** rules for a given node —
is delivered as a **separate, per-foundry plugin** under that foundry's NDA,
never in this repository.

```text
  vyges-em-ir — OPEN engine  (Apache-2.0, contains no fab data)
  ────────────────────────────────────────────────────────────────────
    PDN network (pads · resistors · loads)  ─►  solve V  ─►  IR drop / EM
                                              ▲
                                              └─ published plugin contract
                                                 (per-layer EM limits · thermal rules)
                                       │
        ┌──────────────────────────────┴──────────────────────────────┐
  OPEN reference plugin                          CERTIFIED per-fab plugins
  (in-repo · no NDA)                             (private · one per fab/node 🔒)
    • generic EM limits in the .pdn                • vyges-em-ir-tsmc28
      ✓ runs out of the box                        • vyges-em-ir-sec28
                                                   EM density + electrothermal, under NDA
```

## Current state (2026-05-30)

v0 does **static (DC) IR drop** via a Gauss-Seidel solve of the conductance
system, plus per-layer **EM current-limit** checks — fully offline, no external
deps, 8 tests green. It is enough to find IR hotspots and over-limit segments on
a described PDN.

The road to sign-off grade builds on the same network model: PDN extraction from
DEF/LEF geometry, dynamic/transient IR (decap + switching current), real EM as
current-density × wire geometry, and electrothermal coupling (the BCD/power axis
— the engine reserves the `EmIrError::ElectrothermalNotModeled` hook).
