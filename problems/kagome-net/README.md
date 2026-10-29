<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Kagome triangle network (F18)

The kagome triangle network covers the chip with edge-disjoint triangles that share corners, the geometry of a kagome lattice. Each triangle penalizes the all-equal states, so the ground states obey a not-all-equal rule. The family was screened as a frustrated, ice-like structure with a simple local rule. Classical solvers close it.

## Construction
The strong layer is an edge-disjoint packing of antiferromagnetic triangles at `+1` (`+1000` milli, in units of the full-strength coupler), each made of one odd coupler plus two internal couplers, with no gauge. Every spin sits in at most two triangles, so triangles share corners. The weak layer sets every other coupler to `+0.125` or `-0.125` (`±125` milli). Fields are 0. The cell generator derives the packing and the weak signs from the nonce. The first nonce has 2,674 triangles, 8,022 strong couplers (three per triangle) and 33,452 weak couplers. Each triangle has six ground states, the not-all-equal states, and the weak layer sets the barrier between low states.

Topology use: 100% of qubits, 100% of couplers, 100% of the topology.

## Compact representations
- Problem: 256 bits (nonce, plus the generator version). Each triangle costs 0.500[(sum s)^2 − 3].
- Solution: 4,575 bits raw, one per used qubit. The compact form is not measured. Each triangle takes one of 6 of 8 states (not all equal).
- Compact formula: yes. Six of eight states sit at `-1` unit with a gap of 4. A 6-spin block covers two adjacent triangles.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `kagome-net` | triangle bonds at 1, other bonds `±0.125` | [`corpora/kagome-net.jsonl`](../../corpora/kagome-net.jsonl) | Closed | 4 (suite tier 2) | Tier 1 |

## Generator and postprocess
The generator and the postprocess are not imported. No source file in `sources/` belongs to this family. The research generator is `generate.py` in the research cell directory `qpu-suite/work/cells/kagome-net`. The production postprocess is `dimers` plus `sa`, 0.76 s of one core per call, with a median gain of 3 (1) energy units.

## Results
- Strongest attack: 12 / 0 / 0, GPU single-flip at 8,192 sweeps, 8.2 s per model. CPU six-spin blocks at 8,192 sweeps also win 12 of 12 at every setting.
- Rank: QPU (0.046 s) < CUDA (0.619 s) < CPU (1.90 s). Both CPU and CUDA beat the QPU.
- Best classical: CUDA 0.8× (0.4×). CPU gap: 2.4× (1.3×).
- Energy gap: `+6` (3), QPU 0 / tie 0 / CPU 30, 7.71 s, 30 models.
- GSE medians in energy units are QPU `-4,057`, CUDA `-4,055`, CPU `-4,067`.
- Deep sweep: CUDA 4 (3) / 16,384 sweeps / 1 s.
- New-pow adversary: 11 / 1 / 0 of 12, single-flip population anneal (glasssa) at 2,048 sweeps, 0.053 s wall, ratio vs access 1.16.
- QPU deep captures (48 reads each, 12 models): 200 µs at 0.052 s per call reaches a median of `-3` energy units below the production target (11 of 12 below it). 500 µs at 0.066 s reaches `-5` (12 of 12). 2,000 µs at 0.138 s reaches `-8` (12 of 12). QPU access used in total is 3.63 s. Single-flip GPU at 131,072 sweeps on a shared card wins 12 of 12.
- Class: 1, classical beats cheaply. Profile: classical cheap, CPU competitive.
- Run counts (QPU / CUDA / CPU): 48 / 72 / 360.
- Closed by: exact six-spin blocks on the CPU at 8,192 sweeps.

## Limits and tiling
Tileable: yes. All three tiled-generator conditions pass across 12 nonces. The protocol composes 4 hash-chained tiles. The largest absolute energy delta is 2.9375 units per tile, with tiled wins / ties / direct wins of 11 / 0 / 1. Seam strength is 0.125 and the total satisfied-seam benefit is 0.500 units. Chaining adds nothing. A single-flip anneal beats the call per link in 0.085 s (tier 1). One CPU job needs 0.093 s per link, and a fleet needs 0.086 s at 100 or 1,000 jobs.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories: `regimes/qpu-suite/work/cells/kagome-net`, `regimes/new-pow/kagome-net`, `regimes/new-pow/tiled-generator/kagome-net`.
