<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Strong bond network (F2)

A strong bond network marks a random subgraph of the chip couplers as strong, with random sign and size 0.800 to 1. A weak glass at 0.200 fills the rest. The family has three mean strong degrees and covers the full chip. It was screened because the strong subgraph stays close to a forest, which an exact local solver could exploit.

## Construction
Each native coupler joins the strong subgraph with probability d / 18.1, where d is the mean strong degree (1.5, 2, or 3). One shared nonce-seeded uniform draw makes the three density masks nested. A strong coupler has a random sign and a size from 0.800 to 1 (`800` to `1000` milli). Every other native coupler keeps the sign of its protocol draw at size 0.200 (`200` milli), a ratio of 5. All fields are 0. The nonce fixes the strong mask, the signs, and the sizes.

| Cell | Strong couplers in the first nonce | Other facts |
| --- | --- | --- |
| `strong-net-d1.5` | 3,489 strong and 37,985 weak. Across models: 3,310 to 3,506 strong. | Cycle rank of the strong subgraph is 135 to 215, so the strong graph stays close to a forest. |
| `strong-net-d2` | 4,572 strong (2,339 positive, 2,233 negative) and 36,902 weak. Across 24 models: 4,507 to 4,655 strong. | The giant component has about 3,700 spins. |
| `strong-net-d3` | 6,894 strong and 34,580 weak. | Mean strong degree about 3. |

A spin has about 15 weak neighbours and a typical weak field of 0.825, so the weak layer is large enough to matter. Each cell uses 100 percent of qubits (4,575 of 4,575) and 100 percent of couplers (41,474 of 41,474). The cells have 404 distinct coupling values.

## Compact representations
- Problem: 256 bits (the nonce, plus the generator version). It describes a random strong subgraph of density d / 18.1. The explicit form of the largest model is 41,474 couplers at 9 bits each, which is 373,266 bits, because 404 values occur.
- Solution: none beyond the tree-like parts of the strong network. The compact solution size is not measured. The raw form is 4,575 bits, one per used qubit.

Worked example for `strong-net-d2` (nonce `e9717485...`): spins 2298, 139 and 2309 form a strong triangle with couplings `+0.836`, `-0.910` and `+0.953`. The product of the signs is negative, so no state satisfies all three bonds. Spin 2298 also has a strong bond of `-0.934` to spin 0 and seven weak couplers at `+-0.200`.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `strong-net-d1.5` | Mean strong degree 1.5, 0.200 glass | [`corpora/strong-net-d1.5.jsonl`](../../corpora/strong-net-d1.5.jsonl) | Closed | 3 | Tier 2 |
| `strong-net-d2` | Mean strong degree 2, 0.200 glass | [`corpora/strong-net-d2.jsonl`](../../corpora/strong-net-d2.jsonl) | Closed | 4 | Tier 2 |
| `strong-net-d3` | Mean strong degree 3, 0.200 glass | [`corpora/strong-net-d3.jsonl`](../../corpora/strong-net-d3.jsonl) | Closed | 4 | Tier 2 |

## Generator and postprocess
- Generator: [`strong_net.py`](../../tools/structured/isingmark_structured/families/strong_net.py). It ports the suite generator `work/strongnet.py`. One uniform draw per native coupler marks it strong when below d / 18.1, so the three density masks are nested. One protocol draw from the tri-match alphabet covers every coupler. Strong values stay, and weak couplers keep their sign at 200 milli. The shared alphabet comes from [`tri_match.py`](../../tools/structured/isingmark_structured/families/tri_match.py). Cell parameters: [`params/`](params/).
- Postprocess: the table values apply the `dimers` and `sa` postprocess. It runs for 0.25 s of one core per call for `d1.5` and `d3`, and 0.76 s for `d2`. The code is not imported. No source file is owned by this family under `sources/`.

## Results
All three cells are Closed, class 1 (classical beats cheaply), profile classical cheap and CPU competitive. The QPU call is 48 reads at 80 us and costs 0.046 s. Both CPU and CUDA beat the QPU in every cell.

| Cell | Best classical | Rank | Energy gap | Strongest attack (classical / tie / QPU) |
| --- | --- | --- | --- | --- |
| `strong-net-d1.5` | CUDA 15x (4.4x) | QPU (0.046 s) < CUDA (4.49 s) < CPU (32.7 s) | `+7` (4), QPU 2 / tie 0 / CUDA 98, 4.49 s, 100 models | 11 / 0 / 1, GPU single-flip, 32,768 sweeps, 14.5 s (131,072 sweeps: 12 / 0 / 0, 61.7 s) |
| `strong-net-d2` | CUDA 1.5x (0.8x) | QPU (0.046 s) < CUDA (1.22 s) < CPU (13.4 s) | `+6` (5), QPU 3 / tie 0 / CUDA 27, 2.01 s, 30 models | 12 / 0 / 0, GPU single-flip, 131,072 sweeps, 60.7 s |
| `strong-net-d3` | CUDA 3.4x (1.0x) | QPU (0.046 s) < CUDA (1.00 s) < CPU (10.3 s) | `+14` (6), QPU 0 / tie 0 / CUDA 30, 4.12 s, 30 models | 12 / 0 / 0, GPU single-flip, 131,072 sweeps, 60.7 s |

Energy gaps are in energy units. The ratios compare time to the QPU access time.

- Closed by: exact 6-spin blocks on the CPU and the single-flip GPU. At 8,192 sweeps the 6-spin blocks beat the production call on 11 of 12 models for `d1.5` (gap `-6,401`), 11 of 12 for `d2` (gap `-8,276`), and 10 of 12 for `d3` (gap `-8,463`). At 2,000 us the QPU result against the blocks is 6 / 0 / 6, 7 / 0 / 5, and 8 / 0 / 4 (classical / tie / QPU).
- Single-flip GPU ladder, first rung with a majority: 16,384 sweeps wins 10 of 12 models (`d1.5`), 9 of 12 (`d2`), and 11 of 12 (`d3`). At 8,192 sweeps the GPU wins 1 of 12, 5 of 12, and 5 of 12. The reliable 100-model rung for `d1.5` is 65,536 sweeps, which wins 98 of 100 (4.49 s per model). The 30-model rung for `d3` at 65,536 sweeps wins 30 of 30 (4.12 s).
- New-pow verdict: a single-flip Metropolis anneal at 16,384 sweeps wins 8 / 0 / 4 of 12 (`d1.5`, 1.86 s wall), 10 / 0 / 2 of 12 (`d2`, 1.74 s), and 8 / 0 / 4 of 12 (`d3`, 1.69 s).
- Deeper QPU anneals (48 reads each): at 2,000 us the call beats the production target on 12 of 12 models for `d1.5` (median `-5` energy units) and `d3` (median `-6`), and on 11 of 12 for `d2` (median `-6`).
- Median energies after postprocess over 12 models, in energy units. `d1.5` has QPU `-4,456`, CUDA `-4,461`, CPU `-4,463`. `d2`: QPU `-4,900`, CUDA `-4,900`, CPU `-4,912`. `d3`: QPU `-5,706`, CUDA `-5,706`, CPU `-5,720`.
- CPU depth: the 120 s rung reaches a median of `-12`, `-16`, and `-18` energy units below the QPU target (`d1.5`, `d2`, `d3`) on 12 of 12 models. The 1,200 s rung covers 1 of 12 models (`-15` each).
- The full-cohort crossing for the GPU is not measured.

## Limits and tiling
- Tileable: yes, for all three cells. The tiled-generator protocol composes 4 hash-chained tiles as a grid with a 30.0 s budget per tile and an equal total direct-solve budget. All three conditions pass across 12 nonces.
  - `d1.5`: the largest energy delta is 6.3265 units per tile. Tiled wins / ties / direct wins are 11 / 0 / 1. Tile solvers reached 19 of 48 targets.
  - `d2`: the largest delta is 6.5415 units per tile. Tiled wins / ties / direct wins are 9 / 0 / 3. Tile solvers reached 29 of 48 targets.
  - `d3`: the largest delta is 5.343 units per tile. Tiled wins / ties / direct wins are 9 / 0 / 3. Tile solvers reached 37 of 48 targets.
  - The seam strength is 0.200 and the seam benefit B is 0.800. The tiled and direct CPU totals match to within 1 percent.
- Chaining adds nothing. A single-flip anneal matches the QPU per link in 1.7 to 1.9 s on 6 threads. One CPU job needs 2.57 s per link (`d1.5`), 2.02 s (`d2`), and 2.13 s (`d3`). A fleet of 100 or 1,000 jobs needs 1.87 s, 1.73 s, and 1.71 s per link.
- The strong subgraph has no closed-form answer beyond its tree-like parts. The weak glass has none either.
- Verdict CPU wall by density: 1.86 s (8 of 12), 1.74 s (10 of 12), and 1.69 s (8 of 12) for densities 1.5, 2, and 3. The call-to-call spread of the QPU is not measured, because no repeat calls exist.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories: `qpu-suite/work/cells/strong-net-d1.5`, `qpu-suite/work/cells/strong-net-d2`, `qpu-suite/work/cells/strong-net-d3`, `qpu-suite/qpu/strong-net-d1.5`, `qpu-suite/qpu/strong-net-d2`, `qpu-suite/qpu/strong-net-d3`, `new-pow/strong-net-d1.5`, `new-pow/strong-net-d2`, `new-pow/strong-net-d3`, `new-pow/gpu/measurements`, `new-pow/tiled-generator`.
