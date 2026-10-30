<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Strong pairs over a glass (F6, F10)

The family places a matching of strong ferromagnetic pairs (dimers) at `-1` over a weak glass across the full chip. The weak scale is set by a ratio to the strong pairs. The F10 cells are the crossover cells at fixed ratios 5, 6, and 7. The F6 cell mixes ratios per pair over a power-law background. The family was screened to find the ratio at which the QPU beats pair-aware classical moves.

## Construction
Each cell has a strong layer of ferromagnetic pairs at `-1` (`-1000` milli) and a weak layer on every other coupler. All fields are 0. Each pair aligns its two spins, so contraction of each pair to one spin leaves a reduced glass.

| Cell | Strong layer | Weak layer | Other facts |
| --- | --- | --- | --- |
| `dimer-r5` | 2,239 pairs at `-1` | 39,235 couplers at `+-0.200`, ratio 5 | The round 4 dimer-r generator at ratio 5. The background field on one pair member can exceed the pair bond, so the best states break some pairs. 3 coupling values occur. |
| `dimer-r6` | 2,239 pairs at `-1` | 19,647 couplers at `-0.167` and 19,588 at `+0.167`, ratio 6 | The round 4 dimer-r generator at ratio 6. The weak couplers around one pair member can sum to more than 1, so the best states sometimes break a pair. |
| `dimer-r7` | A perfect matching of 2,239 pairs at `-1`, taken by the native topology-order greedy matching. It leaves 97 spins unmatched. | 39,235 couplers at `+-0.143`, ratio 6.99 | The matching follows the chip topology, so the nonce fixes only the weak signs. Every best state found has all pairs aligned, because a pair break costs 2 units. Contraction gives a `+-J` glass on 2,336 spins and about 35,500 merged couplers. |
| `dimer-power-mixed` | 2,165 pairs at `-1` | 39,309 couplers from a power-law glass, magnitudes 0.010 to 0.820, median 0.040 | Each pair draws a ratio of 4, 5, 6, or 8 (counts 531, 561, 579, and 494 in the first nonce). Every other spin draws a ratio too. The weak coupler between two spins is divided by the geometric mean of their ratios. 162 coupling values occur. |

Each cell uses 100 percent of qubits (4,575 of 4,575) and 100 percent of couplers (41,474 of 41,474).

## Compact representations
- Problem: 256 bits (the nonce, plus the generator version). It describes strong bonds on a matching over a weak background. The explicit form of the largest model is 41,474 couplers at 2 bits each (82,948 bits) for `r5`, `r6`, and `r7`, because 3 values occur. It is 8 bits each (331,792 bits) for `dimer-power-mixed`.
- Solution: partly. The strong layer fixes `s_a s_b = -sign(J)` for each pair, a 2^pairs sector. Contraction to pair spins gives the reduced glass, which has no closed form. The compact solution size is not measured. The raw form is 4,575 bits, one per used qubit.

In the worked example for `dimer-r6` (nonce `e9717485...`), the first pair joins spins 0 and 1 at `-1`. Spin 0 has 9 weak couplers, among them `+0.167` to spin 12 and `-0.167` to spin 2298. Spin 1 has 11, among them `+0.167` to spin 12 and `-0.167` to spin 2.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `dimer-r5` | Pairs at `-1`, background `+-0.200` (ratio 5) | [`corpora/dimer-r5.jsonl`](../../corpora/dimer-r5.jsonl) | Closed | 4 | Tier 2 |
| `dimer-power-mixed` | Pairs at `-1`, ratios {4, 5, 6, 8}, POWER-125 background | [`corpora/dimer-power-mixed.jsonl`](../../corpora/dimer-power-mixed.jsonl) | Closed | 4 | Tier 1 |
| `dimer-r6` | Pairs at `-1`, background `+-0.167` (ratio 6) | [`corpora/dimer-r6.jsonl`](../../corpora/dimer-r6.jsonl) | Closed | 4 | Tier 1 |
| `dimer-r7` | Pairs at `-1`, background `+-0.143` (ratio 7) | [`corpora/dimer-r7.jsonl`](../../corpora/dimer-r7.jsonl) | Closed | 4 | Tier 1 |

## Generator and postprocess
- `dimer-power-mixed`: [`dimer_power_mixed.py`](../../tools/structured/isingmark_structured/families/dimer_power_mixed.py). It ports the suite generator `work/mixed_power.py`. A greedy matching on a random edge order fixes the dimers at `-1000` milli. Each dimer and each unmatched spin draws a ratio from 4, 5, 6, and 8. The background couplers take a POWER-125 protocol draw, rescaled by the geometric mean of their endpoint ratios and rounded to the nearest 10 milli with a floor of 10. Cell parameters: [`params/dimer-power-mixed.json`](params/dimer-power-mixed.json).
- `dimer-r5`, `dimer-r6`, `dimer-r7`: generator not imported. The research generator is the round 4 dimer-r generator at ratios 5, 6, and 7. Its per-cell script is `work/cells/<cell>/generate.py` in the `qpu-suite` research directory.
- Postprocess: the table values apply the `dimers` postprocess with a `sanat` anneal (`dimer-r5`, `dimer-power-mixed`, `dimer-r6`) or a `csa` anneal (`dimer-r7`). It runs for 0.25 s of one core per call for `dimer-r5` and 0.76 s for the other three. The code is not imported. No source file is owned by this family under `sources/`.

## Results
All cells are Closed, class 1 (classical beats cheaply), profile classical cheap and CPU competitive. The QPU call is 48 reads at 80 us and costs 0.046 s.

| Cell | Best classical | Rank | Energy gap | Strongest attack (classical / tie / QPU) |
| --- | --- | --- | --- | --- |
| `dimer-r5` | CPU 2.0x (0.1x) | QPU (0.046 s) < CPU (0.597 s) < CUDA (20.6 s) | `+5` (4), QPU 3 / tie 0 / CPU 27, 182 s, 30 models | 11 / 0 / 1, CPU pair-weighted flips, 8,192 sweeps, 9.6 s (GPU 131,072 sweeps: 11 / 0 / 1, 57.6 s) |
| `dimer-power-mixed` | CPU 1.8x (0.9x) | QPU (0.046 s) < CPU (1.44 s) < CUDA (40.3 s) | `+6` (3), QPU 0 / tie 0 / CPU 30, 124 s, 30 models | 11 / 0 / 1, CPU reduction and polish, 8,192 sweeps, 2.0 s (GPU 131,072 sweeps: 11 / 0 / 1, 61.1 s) |
| `dimer-r6` | CPU 3.0x (1.6x) | QPU (0.046 s) < CPU (2.41 s) < CUDA (>8.25 s) | `+4` (3), QPU 3 / tie 0 / CPU 27, 154 s, 30 models | 11 / 0 / 1, CPU pair-weighted flips, 8,192 sweeps, about 10 s (GPU 131,072 sweeps: 7 / 0 / 5, 57.9 s) |
| `dimer-r7` | CPU 0.3x (0.0x) | QPU (0.046 s) < CPU (0.244 s) < CUDA (44.1 s) | `+4` (3), QPU 2 / tie 0 / CPU 28, 178 s, 30 models | 12 / 0 / 0, CPU reduction and polish, 8,192 sweeps, 2.9 s (GPU 131,072 sweeps: 1 / 0 / 11, 59.9 s) |

Energy gaps are in energy units. The ratios compare time to the QPU access time.

- CPU beats CUDA in `dimer-r7`. Both beat the QPU in the other three cells. In `dimer-r7` the single-flip GPU at 131,072 sweeps loses to the production call (the QPU wins 11 of 12).
- Closed by: weighted pair flips on the CPU at 8,192 sweeps. The single-flip GPU also closes `dimer-power-mixed`.
  - `dimer-r5`: CPU pair-weighted flips win 11 of 12 at every captured setting (9.6 s per model). The production call wins 1 of 12 against the single-flip GPU at 131,072 sweeps.
  - `dimer-r6`: CPU pair flips win 11 of 12 at every captured setting. CPU reduction and polish at 8,192 sweeps wins 9 of 12 (2.9 s per model). The production call wins 5 of 12 against the single-flip GPU at 131,072 sweeps.
  - `dimer-r7`: CPU pair flips win 12 of 12 at every captured setting.
  - `dimer-power-mixed`: weighted pair flips at 8,192 sweeps. The production call wins 0 of 12.
- The single-flip GPU needs a deep ladder. Idle-card ladder on 12 models, first rung with a majority at or below the target: 32,768 sweeps for `r5` (8 of 12), 65,536 sweeps for `power-mixed` (8 of 12), 131,072 sweeps for `r6` (7 of 12), and 524,288 sweeps for `r7` (8 of 12). The reliable rungs are 262,144 sweeps on 100 models for `r5` (87 wins, 6 ties of 100, 20.6 s), 524,288 sweeps on 30 models for `power-mixed` (27 of 30, 40.3 s), and 2,097,152 sweeps for `r7` (12 of 12, 164 s). `r6` has no reliable GPU rung (131,072 sweeps wins 9 of 30).
- CPU time to the target: `r5` 0.597 core-seconds (0.1 s wall on 6 threads), `power-mixed` 1.44 s, `r6` 2.41 s, and `r7` 0.244 core-seconds (0.041 s wall on 6 threads).
- New-pow verdict: pair contraction, anneal, and pair moves win 6 / 1 / 5 of 12 (`r5`, 0.05 s wall), 9 / 1 / 2 (`power-mixed`, 0.053 s), 7 / 1 / 4 (`r6`, 0.051 s), and 10 / 0 / 2 (`r7`, 0.045 s).
- Median energies after postprocess over 12 models, in energy units. `r5` has QPU `-4,255`, CUDA `-4,261`, CPU `-4,268`. `power-mixed` has QPU `-3,969`, CUDA `-3,971`, CPU `-3,980`. `r6` has QPU `-3,916`, CUDA `-3,916`, CPU `-3,922`. `r7` has QPU `-3,670`, CUDA `-3,675`, CPU `-3,677`.
- Deeper QPU anneals (48 reads each): at 2,000 us the call beats the production target on 7, 11, 7, and 10 of 12 models (`r5`, `power-mixed`, `r6`, `r7`), with medians of `-2`, `-4`, `-1`, and `-1` energy units.
- CPU depth: the 120 s rung reaches a median of `-11`, `-15`, `-8`, and `-7` energy units below the QPU target on 12 of 12 models. The 1,200 s rung covers 1 of 12 models (3 of 12 for `r7`).
- The full-cohort GPU crossing is not measured.

## Limits and tiling
- Tileable: no, for all four cells. Conditions 1 and 3 pass, and condition 2 fails. The tiled-generator protocol composes 4 hash-chained tiles as a grid with a 30.0 s budget per tile. Tiled wins / ties / direct wins are 12 / 0 / 0 in every cell.

  | Cell | Largest energy delta per tile | Seam strength | B | Tile solvers reached |
  | --- | --- | --- | --- | --- |
  | `dimer-r5` | 3.9 units | 0.200 | 0.800 | 43 of 48 targets |
  | `dimer-power-mixed` | 2.985 units | 0.010 | 0.040 | 31 of 48 |
  | `dimer-r6` | 2.676 units | 0.167 | 0.668 | 36 of 48 |
  | `dimer-r7` | 1.3615 units | 0.143 | 0.572 | 37 of 48 |

- Chaining adds nothing. Pair moves or plain anneals beat the call per link at 0.08 to 6 s, so a chain only multiplies the classical lead. One CPU job needs 0.103 s per link (`r5`), 0.085 s (`power-mixed`), 0.109 s (`r6`), and 0.06 s (`r7`). A fleet of 100 or 1,000 jobs needs 0.09, 0.083, 0.093, and 0.051 s per link.
- Verdict CPU wall by pair ratio: 0.05 s for `r5` (7 of 12 models), 0.051 s for `r6` (8 of 12), 0.045 s for `r7` (10 of 12), and 0.053 s for the power-law background (10 of 12).
- The strong-pair layer gives a classical solver a cheap contraction. The single-flip GPU, which does not contract pairs, needs 131,072 to 2,097,152 sweeps. The call-to-call spread of the QPU is not measured, because no repeat calls exist.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories: `qpu-suite/work/cells/dimer-r5`, `qpu-suite/work/cells/dimer-power-mixed`, `qpu-suite/work/cells/dimer-r6`, `qpu-suite/work/cells/dimer-r7`, `qpu-suite/qpu/dimer-r5`, `qpu-suite/qpu/dimer-power-mixed`, `qpu-suite/qpu/dimer-r6`, `qpu-suite/qpu/dimer-r7`, `new-pow/dimer-r5`, `new-pow/dimer-power-mixed`, `new-pow/dimer-r6`, `new-pow/dimer-r7`, `new-pow/retime`, `new-pow/gpu/measurements`, `new-pow/tiled-generator`.
