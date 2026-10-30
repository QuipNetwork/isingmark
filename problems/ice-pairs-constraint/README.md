<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Spin ice with pair constraints, no glass

This family replaces the weak glass of twisted spin ice with planted pair constraints. A matching adds 600 to 1,800 pairs at `-1` on couplers outside the tiles, and a state must keep every pair equal. A state must also meet the ice rule on every tile. Pairs reduce the problem to exact constraints on merged variables. The family was screened to test whether a pure constraint problem stays hard. It did not. A focused random walk with merged pairs reaches the QPU result in 0.092 s.

## Construction
Couplings are in units of the full-strength coupler (J = 1), with the milli value in a code span. The working graph has 4,575 qubits and 41,474 couplers.

- Strong layer: 1,785 K4 tiles, each with its own gauge, so the six bonds of a tile are `+1 g_a g_b`. Tiles share spins, which leaves 4,104 spins inside tiles. This gives 10,710 tile bonds at ±1.
- Pairs: a matching adds 1,350 planted pairs at `-1` on couplers outside the tiles. The pair count P ranges from 600 to 1,800 in the formula.
- No weak layer and no field: every coupler is a tile bond or a pair, all at ±1.
- A state is a ground state when it meets the two-of-four rule on every tile and keeps every pair equal. Pairs reduce the problem to 1,785 exact constraints on about 2,750 merged variables.
- Topology use: 89.9 percent of qubits (4,112 of 4,575), 29.1 percent of couplers (12,060 of 41,474), and 35.1 percent of the topology. Each model has 12,060 couplers, that is 10,710 tile bonds plus 1,350 pairs.

## Compact representations
- Problem (256 bits: the nonce plus the generator version): twisted ice plus `sum_pairs -1 s_a s_b` (P = 600 to 1,800).
- Solution (4,112 bits raw at one bit per used qubit, compact size not measured): ice states with every pair equal. The count is about 2^(1,560 − P), which reaches zero near 1,560 pairs. The integer-solver time agrees with that estimate.
- Explicit size of the largest of 12 models: 12,060 couplers × 1 bit = 12,060 bits (2 values).

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `ice-pairs-constraint-1350` | Twisted ice tiles at ±1 plus 1,350 pairs at −1. No glass and no field. | [`corpora/ice-pairs-constraint-1350.jsonl`](../../corpora/ice-pairs-constraint-1350.jsonl) | Closed | 4 | Tier 1 |

Tier 3 and tier 4 mean that a classical run inside 120 s beats the postprocessed QPU result on at least 60 percent of 30 or more common models. Tier 4 means the best classical run is also within five times the QPU wall time. Tier 1 and tier 2 mean no classical run beats the QPU in that way, and tier 1 means the QPU is also fast. New-pow Tier 1 means classical is strictly lower on a majority of models within 0.1 s. New-pow Tier 2 means parity within 20 min.

## Generator and postprocess
- Instance generator: `new-pow/families/ice-pairs-constraint/generate.py` (research root), which is not imported. The nonce fixes the tile matching. It also fixes the gauges and the pair matching.
- Postprocess: the `ice-pairs` optimizer with method `pw` (a focused random walk with merged pairs), at 0.25 s of one core per call. It gains 3 (2) energy units over greedy descent. Code: [`pairs.py`](../twisted-ice/sources/new-pow/postprocess/optimizers/ice-pairs/pairs.py), [`pairworm.c`](../twisted-ice/sources/new-pow/postprocess/optimizers/ice-pairs/pairworm.c), [`run.py`](../twisted-ice/sources/new-pow/postprocess/optimizers/ice-pairs/run.py), and [`Makefile`](../twisted-ice/sources/new-pow/postprocess/optimizers/ice-pairs/Makefile). These files are owned by the twisted-ice family.

## Results
- Class and profile: not recorded. Tier 4. New-pow Tier 1.
- Energies, median over models, in energy units: QPU −4,903, QPU after postprocess −4,903, CPU −4,920, CUDA −4,900.
- Energy gap: +11 (3), QPU 0 / tie 0 / CPU 30, 1.90 s, 30 models.
- Best classical: CPU at 1.9 times the QPU access time (0.1 times the QPU wall time). The `tier1-n0.22-seed101-m30` run reaches the target at 0.553 core-seconds per model (0.092 s wall on 6 threads). CUDA has no rung that reaches the target on a majority of models. Its deepest rung, a single-flip anneal at 524,288 sweeps over 30 models, needs more than 25.7 card-seconds (more than 87 times the QPU access cost).
- QPU time to target: 0.077 s, from 0.046 s per call and a measured per-call success rate of 0.60 (median over 12 models). The pooled rate is 0.57 (68 of 120 repeat calls). The repeat-call spread of the best result is 8 energy units (4 to 10 over 12 models).
- Rank by time to the postprocessed target: QPU (0.077 s) < CPU (0.553 s) < CUDA (more than 25.7 s).
- New-pow campaign (2026-10-20): 12 / 0 / 0 over 12 models for a focused random walk with merged pairs at noise 0.22. The median wall is 0.092 s, which is 1.99 times the QPU access time. Kissat decides the strong floor of every pair cell in about 0.3 s.
- Verdict date: 2026-10-26.

## Limits and tiling
- Tileable: no. Condition 2 fails: the largest energy delta is 2.5 units per tile, with tiled wins, ties, and direct wins of 12 / 0 / 0.
- Chaining: no historical chaining analysis covers this cell. One fresh call qualifies 60 percent of the time (median per model), so a link costs the QPU 0.167 s at 0.1 s per call.
- Limit of the approach: the constraint count controls the hardness. The solution count falls as 2^(1,560 − P), and an exact integer solver follows that estimate. 
- Depth: at 1,200 s, three models reach a CPU result with a median of 22 energy units below the QPU target (deepest 24).

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories under `/home/carback1/quip-data/regimes`: `new-pow/qpu-recap-mock/work/cells/ice-pairs-constraint-1350`, `new-pow/ice-pairs-constraint-1350`.
