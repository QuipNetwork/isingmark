<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Spin ice with a middle layer (F14)

This family adds a middle coupling layer to the plain spin ice problem. Corner-sharing tetrahedra on the Zephyr working graph carry strong antiferromagnetic bonds. Medium bonds link adjacent tetrahedra. A weak glass fills the remaining couplers. It was screened to test whether a third coupling scale slows the classical solvers that beat the two-layer ice glass. It did not. A single-flip GPU anneal closed it.

## Construction
Couplings are in units of the full-strength coupler (J = 1), with the milli value in a code span. The working graph has 4,575 qubits and 41,474 couplers.

- Strong layer: 1,785 edge-disjoint K4 tetrahedra with every bond at +1 (`+1000`). The cell reuses the F11 tile matching and one global gauge per nonce.
- Middle layer: every remaining internal coupler whose endpoints both sit on tetrahedra, at ±0.350 (`±350`).
- Weak layer: every other coupler at ±0.045 (`±45`).
- One sign stream, seeded by the nonce, signs all non-strong couplers.
- First nonce: 10,710 strong, 22,302 medium, and 8,462 weak couplers, 41,474 in total.
- Fields: h 0. Distinct couplers: {±0.045, ±0.350, ±1}.
- Topology use: 100 percent of qubits and 100 percent of couplers.

## Compact representations
- Problem (256 bits: the nonce plus the generator version): `sum_t 0.500[(sum_{i in t} g_ti s_i)^2 - 4]` over 1,785 tiles, with a global gauge (F11) or a per-tile gauge (F12), plus a glass of ±0.031 to ±0.250. F14 adds a ±0.350 middle layer. The source states this one formula for several families, so a formula for this cell alone is not recorded. The first nonce gives one global gauge, 10,710 strong couplers at 1, 22,302 medium couplers at ±0.350, and 8,462 weak couplers at ±0.045.
- Solution (3,035 bits compact, mean per qualifying state, at most 3,141): the strong layer obeys the ice rule with 6 of 16 states per tile. The Pauling count is about 2^1,560. The strong floor of −3,570 is closed form. The glass optimum has no closed form.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `ice-three-level` | Tetrahedra `+1000`, middle layer `±350`, weak layer `±45` | [`corpora/ice-three-level.jsonl`](../../corpora/ice-three-level.jsonl) | Closed | 3 | Tier 2 |

Tier 3 and tier 4 mean that a classical run inside 120 s beats the postprocessed QPU result on at least 60 percent of 30 or more common models. Tier 4 means the best classical run is also within five times the QPU wall time. Tier 1 and tier 2 mean no classical run beats the QPU in that way, and tier 1 means the QPU is also fast. New-pow Tier 1 means classical is strictly lower on a majority of models within 0.1 s. New-pow Tier 2 means parity within 20 min.

## Generator and postprocess
The instance generator is `qpu-suite/work/ice_three_level.py` (research root), which is not imported. It builds three-scale couplings on the F11 tetrahedron geometry. The F11 base generator is imported at [`ice.py`](../shared/sources/qpu-suite/work/ice.py), and the structured port of the F11 construction is [`ice_glass.py`](../../tools/structured/isingmark_structured/families/ice_glass.py), which covers the two-layer backgrounds only. Postprocess: the shared `ice-tiles` optimizer ([`icetiles.py`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/icetiles.py), [`icetiles.c`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/icetiles.c), [`method.py`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/method.py)), with `anneal:sa=1,b0=2,len=64,fe=16,nat48=1` at 0.25 s of one core per call. It gains 1 (1) energy unit over greedy descent.

## Results
- Class 1 (classical beats the production call cheaply). Profile: classical cheap and CUDA-bound. CUDA beats CPU.
- Energies, median over models, in energy units: QPU −5,949, QPU after postprocess −5,953, CPU −5,960, CUDA −5,967. The best anneal time is 2,000 µs.
- QPU against the best classical attack. A GPU single-flip anneal at 32,768 sweeps rejects every captured setting (14.4 s per model). The GPU beats the production call on 12 of 12 models at 131,072 sweeps.
- Energy gap: +10 (5), QPU 3, tie 0, CUDA 97, 5.55 s, 100 models. Strongest attack: 12 / 0 / 0 for GPU single flip at 32,768 sweeps, 14.4 s.
- Best classical: CUDA at 19 times the QPU access time (5.5 times the QPU wall time). Closed by: single-flip GPU at 32,768 sweeps.
- New-pow campaign (2026-10-20): 9 / 1 / 2 wins, ties, and losses over 12 models for an Otsuka cluster anneal on gauged tetrahedra at 69,632 sweeps. Median wall 18.7 s, which is 406 times the QPU access time.
- QPU time to target: 0.153 s, from 0.046 s per call and a per-call success rate of 0.30 (median over 12 models, pooled 0.38). Repeat-call spread of the best result: 10 energy units (7 to 15). Rank by time to the postprocessed target: QPU (0.153 s) < CUDA (5.55 s) < CPU (433 s).
- Longer QPU anneals help. The 2,000 µs call reaches a median 8 energy units below the production target on 11 of 12 models.
- Verdict date: 2026-10-26.

## Limits and tiling
- Tileable: yes. All three tiled-generator conditions pass across 12 nonces. The largest energy delta is 12.1475 units per tile, with wins, ties, and losses of 11 / 0 / 1. The tile solvers reached 7 of 48 targets.
- Chaining: a hash chain is possible. A GPU single-flip anneal at 32,768 sweeps beats the call on every link, so chaining multiplies a classical lead. A fresh call qualifies 30 percent of the time, so a link costs the QPU 0.333 s at 0.1 s per call. One CPU job needs 30.7 s per link.
- Seed: the CUDA cluster anneal at 8,192 sweeps matches 8 of 12 models on each of two seeds.
- Depth: at 1,200 s one model reaches a CPU result 38 energy units below the QPU.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories under `/home/carback1/quip-data/regimes`: `qpu-suite/work/cells/ice-three-level`, `new-pow/ice-three-level`.
