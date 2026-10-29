<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Stacked twisted ice layers

This family stacks several twisted ice layers on the Zephyr working graph and joins them with vertical bonds. Each qubit belongs to one layer. Each layer carries its own ice tiles. Vertical bonds force the layers to agree. The layers cannot all satisfy their tiles and the vertical bonds together, so the strong layer is frustrated and the ground states keep strong violations. It was screened to test whether that frustration hardens the problem. It did not. A single-flip or cluster GPU anneal reaches the QPU result.

## Construction
Couplings are in units of the full-strength coupler (J = 1), with the milli value in a code span. The working graph has 4,575 qubits and 41,474 couplers.

- `ice-stack-2`: each qubit belongs to one of two layers by the rule k mod 2 on its Zephyr coordinates. Each layer reuses the F11 tile packing, so 1,761 twisted K4 tiles at 1 (`1000`) sit inside the two layers, each with a local gauge g. The six bonds of tile t are `+1 g_a g_b`. A largest matching joins the layers with 2,272 vertical bonds at −1, which reach 99 percent of the qubits. Every other coupler is a weak glass of ±0.062 (`±62`).
- `ice-stack-3`: each qubit belongs to one of three layers by the rule (w + k) mod 3 on its Zephyr coordinates. Each layer reuses the F11 tile packing, so 1,485 twisted K4 tiles at 1 (`1000`) sit inside the three layers. Two largest matchings join layers 0 and 1 and layers 1 and 2 with 2,981 vertical bonds at −1, so most qubits of the middle layer carry two bonds and form columns of three. The glass is ±0.062 (`±62`). Kissat proves the tiles and all vertical bonds unsatisfiable together on all 12 models in 0.01 to 0.48 s, so the strong bound of −5,951 units is out of reach. The expected count of defect-free states is near 2^−507.
- Fields: h 0. Distinct couplers: {±0.062, ±1}. Topology use for `ice-stack-2`: 100 percent of qubits and 100 percent of couplers.
- The nonce orders both matchings and draws the gauges and the glass signs.
- Frustration. Each layer alone has ice states. The SAT solver Kissat proves the tiles and vertical bonds unsatisfiable together on all 12 models of `ice-stack-2` in 0.03 to 0.18 s. It also proves that no state has 2 or fewer strong defects, so the strong bound of −5,794 units is out of reach. A focused walk reaches 76 tile defects. No CPU rung reached a floor.

## Compact representations
- Problem (256 bits: the nonce plus the generator version): layers of twisted ice `sum_t 0.500[(sum_{i in t} g_ti s_i)^2 - 4]` plus vertical bonds `sum_bonds -1 s_a s_b` between adjacent layers and a ±0.062 glass.
- Solution (4,575 bits raw, compact size not measured): stacked ice states that satisfy every vertical bond. Kissat proves this set empty. Ground states keep strong violations (about 160 strong violations for `ice-stack-3`).
- Explicit size of the largest of 12 models of `ice-stack-2`: 41,474 couplers × 2 bits = 82,948 bits (4 values).

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `ice-stack-2` | Two layers, vertical bonds `-1000`, glass `±62` | [`corpora/ice-stack-2.jsonl`](../../corpora/ice-stack-2.jsonl) | Closed | 3 | not measured |
| `ice-stack-3` | Three layers, vertical bonds `-1000`, glass `±62` | [`corpora/ice-stack-3.jsonl`](../../corpora/ice-stack-3.jsonl) | Closed | 3 | not measured |

Tier 3 and tier 4 mean that a classical run inside 120 s beats the postprocessed QPU result on at least 60 percent of 30 or more common models. Tier 4 means the best classical run is also within five times the QPU wall time. Tier 1 and tier 2 mean no classical run beats the QPU in that way, and tier 1 means the QPU is also fast. New-pow Tier 1 means classical is strictly lower on a majority of models within 0.1 s. New-pow Tier 2 means parity within 20 min.

## Generator and postprocess
- Generator: `new-pow/families/ice-stack-2/generate.py` and `new-pow/families/ice-stack-3/generate.py` (research root). Both use framed SHAKE256 draws. They are not imported.
- Postprocess: `ice-tiles` ([`icetiles.py`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/icetiles.py), [`icetiles.c`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/icetiles.c), [`method.py`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/method.py)). For `ice-stack-2` the setting is `anneal:sa=1,pw=1,len=32,b0=2,fe=4,nat48=1` at 0.25 s of one core per call, with a gain of 2 (2) energy units over greedy descent. For `ice-stack-3` the setting is the same, with a gain of 0 (0).

## Results
| Cell | QPU / QPU pp / CPU / CUDA energy (units) | Energy gap | Best classical |
| --- | --- | --- | --- |
| `ice-stack-2` | −5,759 / −5,763 / −5,782 / −5,768 | +11 (6), QPU 1 / tie 0 / CUDA 29, 20.4 s, 30 models | CUDA 35× (10×) |
| `ice-stack-3` | −5,745 / −5,749 / −5,776 / −5,758 | +17 (6), QPU 0 / tie 0 / CPU 30, 120 s, 30 models | CUDA 9.0× (2.6×) |

- Both cells are tier 3. The New-pow verdict field is empty for both cells, although the new-pow measurement directories exist. Class and profile: not recorded. Verdict date: 2026-10-26.
- `ice-stack-2` rank by time to the postprocessed target: QPU (0.046 s) < CUDA (10.2 s) < CPU (16.5 s). The CUDA arm is a cluster anneal at 65,536 sweeps over 30 models and the page marks it not reliable. The reliable rung is 131,072 sweeps (20.4 card-seconds, 29 of 30 models). A single-flip GPU anneal needs 524,288 sweeps for a majority (10 of 12 models, 38.3 card-seconds).
- `ice-stack-3` CPU attack: an anneal with single, column, and tile moves matches 10 of 12 models in a median 1.97 s on 6 threads. Its floor flattens after 120 s, and the QPU best sits about 28 units (14 strong violations) above that floor. The floor is not proved. The CUDA cluster anneal at 16,384 sweeps matches 27 of 30 models in 2.65 card-seconds, and the rank by time to target is QPU (0.046 s) < CUDA (2.65 s) < CPU (7.96 s).
- `ice-stack-2` CPU attack: an anneal with single flips and vertical-pair flips matches 10 of 12 models in a median 3.17 s on 6 threads (about 69 times the 0.046 s call). The CPU gap to the target is 56 times the QPU access cost (16 times the wall cost) at the 30 s rung over 100 models.

## Limits and tiling
- Tileable: yes for `ice-stack-2`. All three tiled-generator conditions pass across 12 nonces, with a largest energy delta of 9.406 units per tile and wins, ties, and losses of 11 / 0 / 1. Tileable for `ice-stack-3`: no. Condition 2 fails with a largest delta of 8.369 units per tile and tiled wins of 12 / 0 / 0.
- Chaining for `ice-stack-2`: a hash chain is possible. Per link the CPU anneal matches in 3.17 s on 6 threads. Seams between chips would cut tiles and vertical bonds and hurt the QPU. For `ice-stack-3` the CPU anneal matches in 1.97 s on 6 threads, 43 times the 0.046 s call.
- Seed results: 44 of 72 model-runs match over six CPU verdict-kernel runs on `ice-stack-2`. For `ice-stack-3`, 46 of 60 match over five runs. Depth: at 1,200 s one model reaches a CPU result 26 energy units below the QPU for `ice-stack-2` and 28 for `ice-stack-3`.
- Limit of the approach: the SAT solver Kissat proves the frustration, yet a cluster GPU anneal at 131,072 sweeps still matches 29 of 30 models of `ice-stack-2`.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories under `/home/carback1/quip-data/regimes`: `new-pow/qpu-recap-mock/work/cells/ice-stack-2`, `new-pow/ice-stack-2`, `new-pow/qpu-recap-mock/work/cells/ice-stack-3`, `new-pow/ice-stack-3`.
