<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Three strong matchings (F1, F9)

The family places three edge-disjoint matchings of strong bonds over a weak glass across the full chip. The strong layer fixes each matched pair up to a global flip, and the weak glass then lives on the contracted pairs. It was screened as a frustrated strong layer with a tunable strong-to-weak ratio.

## Construction
Three edge-disjoint greedy matchings, built in nonce order, own the strong couplers. Strong bonds have random sign and size 0.800 to 1 (`800` to `1000` milli). The nonce seeds a random edge order through sha256. A greedy pass over that order builds the first matching, then the second, then the third. All fields are 0. The strong graph has degree at most 3 and about 1,975 independent cycles, half of them frustrated. Each spin has about 15 weak neighbours, so the weak layer is not negligible.

For the first nonce (`e9717485...`), the matchings hold 2,189, 2,170 and 2,177 couplers. That is 6,536 strong couplers (6,535 to 6,562 across models) and 34,938 weak couplers, which add up to 41,474.

| Cell | Weak layer |
| --- | --- |
| `tri-match-r4` | Every other coupler keeps its drawn sign at `+-0.250` (`250` milli), a ratio of 4. |
| `tri-match-r6` | The same strong layer as `tri-match-r4` for the same nonce. The weak layer is `+-0.167` (`167` milli), a ratio of 6. |
| `tri-match-power` | F1 at ratio 5 over a power-law background (POWER-125) scaled as in the dimer family. The weak sizes run from 0.010 to 0.660 and give each spin a random field of about 0.750 RMS, close to the strong scale. It is the equal-structure control for the dimer family (F6). 534 distinct coupling values occur. |

Each cell uses 100 percent of qubits (4,575 of 4,575) and 100 percent of couplers (41,474 of 41,474). The `r4` and `r6` cells have 404 distinct coupling values.

## Compact representations
- Problem size is 256 bits (the nonce, plus the generator version). It describes strong bonds of 0.800 to 1 on matchings, over a weak background. The explicit form of the largest model is 41,474 couplers at 9 bits each (373,266 bits) for `r4` and `r6`, and at 10 bits each (414,740 bits) for `tri-match-power`.
- Solution: partly. The strong layer fixes `s_a s_b = -sign(J)` for each pair, a 2^pairs sector. Contraction to pair spins gives the reduced glass, which has no closed form. The compact solution size is not measured. The raw form is 4,575 bits, one per used qubit.

In the worked example (nonce `e9717485...`), spin 0 has one bond from each matching, with a bond of `-0.910` to spin 2462, `+0.881` to spin 12, and `+0.852` to spin 1.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `tri-match-power` | Ratio 5, POWER-125 background | [`corpora/tri-match-power.jsonl`](../../corpora/tri-match-power.jsonl) | Closed | 4 | Tier 2 |
| `tri-match-r6` | Background `+-0.167`, ratio 6 | [`corpora/tri-match-r6.jsonl`](../../corpora/tri-match-r6.jsonl) | Closed | 3 | Tier 2 |
| `tri-match-r4` | Background `+-0.250`, ratio 4 | [`corpora/tri-match-r4.jsonl`](../../corpora/tri-match-r4.jsonl) | Closed | 3 | Tier 2 |

## Generator and postprocess
- Generator: [`tri_match.py`](../../tools/structured/isingmark_structured/families/tri_match.py). It ports the suite generator `work/trimatch.py`. Three greedy maximal matchings on fresh random edge orders own the strong couplers. One protocol draw from `[-1000, -800]` and `[800, 1000]` covers every native coupler. Strong values stay. Every other coupler keeps its sign at the background magnitude. The structured generator covers ratios 4 and 6. Cell parameters: [`params/`](params/).
- `tri-match-power`: generator not imported. The research generator is `work/cells/tri-match-power/generate.py` in the `qpu-suite` research directory.
- Postprocess: the table values apply the `dimers` and `sa` postprocess. It runs for 0.25 s of one core per call for `r4` and `r6`, and 0.76 s for `tri-match-power`. The code is not imported. No source file is owned by this family under `sources/`.

## Results
All cells are Closed, class 1 (classical beats cheaply), profile classical cheap and CUDA-bound. The QPU call is 48 reads at 80 us and costs 0.046 s. CUDA beats CPU in every cell, and both beat the QPU.

| Cell | Best classical | Rank | Energy gap | Strongest attack (classical / tie / QPU) |
| --- | --- | --- | --- | --- |
| `tri-match-r4` | CUDA 7.1x (2.1x) | QPU (0.046 s) < CUDA (2.09 s) < CPU (196 s) | `+9` (5), QPU 1 / tie 0 / CUDA 29, 9.45 s, 30 models | 11 / 0 / 1, GPU single-flip, 32,768 sweeps, 14.4 s (131,072 sweeps: 12 / 0 / 0, 58.5 s) |
| `tri-match-r6` | CUDA 34x (10x) | QPU (0.046 s) < CUDA (10.1 s) < CPU (22.0 s) | `+6` (3), QPU 2 / tie 0 / CUDA 98, 10.1 s, 100 models | 12 / 0 / 0, GPU single-flip, 32,768 sweeps, 14.5 s |
| `tri-match-power` | CUDA 3.0x (1.6x) | QPU (0.046 s) < CUDA (2.43 s) < CPU (27.0 s) | `+5` (5), QPU 3 / tie 0 / CUDA 27, 4.24 s, 30 models | 12 / 0 / 0, GPU single-flip, 131,072 sweeps, 130.1 s (32,768 sweeps: 9 wins of 12, 30.6 s) |

Energy gaps are in energy units. The ratios compare time to the QPU access time.

- Closed by: the single-flip weighted GPU kernel at 131,072 sweeps (`r4`, `r6`) and the single-flip GPU at 32,768 sweeps (`tri-match-power`).
- Star updates at 8,192 sweeps do not close `r4` and `r6`. The QPU wins 9 of 12 at 80 us and 11 of 12 at 2,000 us for `r4` (gap `+3,694` and `+6,938`). It wins 10 of 12 at 80 us and 11 of 12 at 2,000 us for `r6`. The GPU gate fails. For `tri-match-power`, GPU at 32,768 sweeps rejects every setting (30.6 s per model), and CPU blocks reject none.
- Single-flip GPU ladder on the idle card, 12 models: 8,192 sweeps wins 4, 0, and 0 of 12 (`r4`, `r6`, `power`). The first rung with a majority is 32,768 sweeps for all three cells (11, 12, and 9 of 12). The 100-model rungs are reliable at 131,072 sweeps for `r6` (98 of 100, 10.1 s) and `r4` (29 of 30, 9.45 s), and at 65,536 sweeps for `power` (27 of 30, 4.24 s).
- New-pow verdict: a single-flip Metropolis anneal wins 8 / 0 / 4 of 12 at 32,768 sweeps (`r4`, 3.27 s wall), 10 / 0 / 2 of 12 at 65,536 sweeps (`r6`, 5.93 s wall), and 10 / 0 / 2 of 12 with a knee schedule at 32,768 sweeps (`power`, 4.51 s wall).
- Median energies after postprocess over 12 models, in energy units. `r4` has QPU `-6,056`, CUDA `-6,059`, CPU `-6,065`. `r6` has QPU `-5,516`, CUDA `-5,521`, CPU `-5,531`. `power` has QPU `-5,698`, CUDA `-5,700`, CPU `-5,711`.
- Deeper QPU anneals (48 reads each): at 2,000 us the call beats the production target on 11 of 12 models for `r4`, 10 of 12 for `r6`, and 10 of 12 for `power`, with medians of `-5`, `-6`, and `-4` energy units.
- CPU depth: the 120 s rung reaches a median of `-10`, `-12`, and `-15` energy units below the QPU target on 12 of 12 models. The 1,200 s rung covers 1 of 12 models (`-12`, `-28`, and `-17`).
- The full-cohort GPU crossing is not measured.

## Limits and tiling
- Tileable: yes, for all three cells. The tiled-generator protocol composes 4 hash-chained tiles as a grid with a 30.0 s budget per tile. All three conditions pass across 12 nonces.
  - `r4`: the largest energy delta is 5.537 units per tile. Tiled wins / ties / direct wins are 11 / 0 / 1. Seam strength is 0.250, and B is 1.
  - `r6`: the largest delta is 5.573 units per tile. Tiled wins / ties / direct wins are 8 / 0 / 4. Seam strength is 0.167, and B is 0.668.
  - `power`: the largest delta is 7.3495 units per tile. Tiled wins / ties / direct wins are 11 / 0 / 1. Seam strength is 0.010, and B is 0.040.
  - Tile solvers reached 35, 36, and 35 of 48 targets.
- Chaining adds nothing. Pair moves or plain anneals beat the call per link at 0.08 to 6 s, so a chain only multiplies the classical lead. One CPU job needs 3.21 s per link (`r4`), 4.21 s (`r6`), and 3.52 s (`power`). A fleet of 100 or 1,000 jobs needs 2.13 s, 1.50 s, and 1.47 s per link.
- Verdict CPU wall by coupling scale: 3.27 s for `r4` (8 of 12), 5.93 s for `r6` (10 of 12), and 4.51 s for the power-law background (10 of 12).
- The reduced glass after pair contraction has no closed form. The call-to-call spread of the QPU is not measured, because no repeat calls exist.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories: `qpu-suite/work/cells/tri-match-power`, `qpu-suite/work/cells/tri-match-r4`, `qpu-suite/work/cells/tri-match-r6`, `qpu-suite/qpu/tri-match-power`, `qpu-suite/qpu/tri-match-r4`, `qpu-suite/qpu/tri-match-r6`, `new-pow/tri-match-power`, `new-pow/tri-match-r4`, `new-pow/tri-match-r6`, `new-pow/gpu/measurements`, `new-pow/tiled-generator`.
