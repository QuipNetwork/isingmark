<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Deceptive cluster loops (F5, F5b)

Deceptive cluster loops turn intact complete bipartite cells of a Chimera sublattice into rigid ferromagnetic clusters. Frustrated loops on the cell lattice link the clusters through bundles of parallel couplers. A random gauge hides the construction. The family was screened because the loop lattice is planar after cell contraction, and a planar lattice is exactly solvable.

## Construction
- `dcl-k44-l5` (lambda 5): a `chimera(24,24,4)` sublattice of the chip, with a 24 by 24 lattice of K4,4 cells. Frustrated loops on the cell lattice (alpha 0.24, R 1) use four parallel inter-cell couplers at `+-1` (`+-1000` milli), one per qubit index. Each loop adds `-1` to its edges and `+2` to one edge, and the generator rejects a loop that pushes any edge sum outside [-1, 1]. Intra-cell couplers are `-0.200` (`-200` milli), so the strong-to-weak ratio is 5. Every other coupler and every field is 0. The working graph drops cells that lose a qubit.
- `dcl-k44-l7` (lambda 7): the same construction with intra-cell couplers at `-0.143` (`-143` milli), which is `-1` divided by the cluster ratio 7.
- `dcl-k88` (F5b): the K8,8 variant on the live Zephyr graph, built as a `chimera(12,12,8)` sublattice. Each retained cell holds 16 working qubits and its working internal couplers (61 to 64 of 64) at `-0.100`. Each retained link between adjacent cells holds eight physical couplers at the loop weight times `0.500`. Link weights stay within `+-2`, so a stacked link sits at `+-1` (the limit of the capture worker). The strong-to-weak ratio is 5 on a single link and 10 on a stacked link. The nonce lays 24 loops per 100 cells. The generator selects offset (4, 0, 0, 0, 0) with 80 cells and 82 links in 11 components (the largest holds 51 cells), and the cell graph is planar. Each model has 19 loops, 1,280 qubits, and 5,245 to 5,413 couplers. An earlier version of the cell was rejected at the construction gate across all four supplied mappings.

K4,4 cells use 65.4 to 65.7 percent of qubits (3,008 of 4,575) and 17.1 to 17.2 percent of couplers (7,136 of 41,474). `dcl-k88` uses 28 percent of qubits (1,280 of 4,575) and 13 percent of couplers (5,405 of 41,474). Topology use is 22 percent (K4,4) and 14.5 percent (K8,8).

## Compact representations
- Problem size is 256 bits (the nonce, plus the generator version). For K4,4 cells, every intra-cell coupler is at `-0.200` (lambda 5) or `-0.143` (lambda 7), plus frustrated loops at density 0.24 per cell on the 24×24 cell lattice. Each loop edge is 4 parallel couplers of `+-1`. For K8,8 cells, every working intra-cell coupler is at `-0.100`, plus 19 frustrated loops on the 80-cell lattice, each loop edge as 8 parallel couplers of `+-0.500`, doubled to `+-1` where two loops share a link. The explicit form is 2 bits per coupler: 14,272 bits (K4,4, 7,136 couplers) and 10,810 bits (K8,8, 5,405 couplers), because 4 values occur.
- Solution: the ground states of a planar loop lattice after cell contraction, with an exact ground energy E0 by elimination. The compact solution size is not measured. The raw form is 3,008 bits (K4,4) and 1,280 bits (K8,8), one per used qubit. For K8,8 the exact E0 holds on 8 of 12 production models. The solver stops on the four models whose elimination width is 31, past its limit of 24.

Worked example for `dcl-k44-l5` (nonce `e9717485...`): the exact solver certifies a ground energy of `-2,086` energy units, 6 units below the QPU best of `-2,080`. For `dcl-k44-l7` the values are `-1,793` and `-1,788`.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `dcl-k44-l7` | K4,4 cells, intra-cell `-0.143` (lambda 7) | [`corpora/dcl-k44-l7.jsonl`](../../corpora/dcl-k44-l7.jsonl) | Closed | 2 | none |
| `dcl-k44-l5` | K4,4 cells, intra-cell `-0.200` (lambda 5) | [`corpora/dcl-k44-l5.jsonl`](../../corpora/dcl-k44-l5.jsonl) | Closed | 2 | none |
| `dcl-k88` | K8,8 cells, intra-cell `-0.100` | [`corpora/dcl-k88.jsonl`](../../corpora/dcl-k88.jsonl) | Closed | 1 | none |

## Generator and postprocess
- K4,4 cells: [`dcl_k44.py`](../../tools/structured/isingmark_structured/families/dcl_k44.py). It ports the suite generator `work/dcl.py`. Intact K4,4 cells of a Chimera sublattice are rigid ferromagnetic clusters at -cell_milli. Random walks on the cell graph plant frustrated loops whose bonds become bundles of four +-1000 milli couplers. A random physical gauge hides the construction. Lambda 5 is 200 milli and lambda 7 is 143 milli. Cell parameters: [`params/dcl-k44-l5.json`](params/dcl-k44-l5.json) and [`params/dcl-k44-l7.json`](params/dcl-k44-l7.json).
- `dcl-k88`: generator not imported. The research generator is `dcl-k88/generate.py` in the `new-pow` research directory. It keeps the placement rule of the K4,4 family. Take the placement with the most cells, then break ties by link count and then by offset. An earlier rule kept only cells with all 64 internal couplers. It gave 55 cells with no cycle, and no placement met the loop count.
- Exact elimination (the attack that closes the family) is a research solver and is not imported. No source file is owned by this family under `sources/`.
- Postprocess: the table values apply the `glasses` and `pt` postprocess for 0.25 s of one core per call. The code is not imported.

## Results
The QPU production capture is 48 reads at 80 us and costs 0.046 s per call (12 calls, 0.553 s in total). The suite rejected the K4,4 cells without a capture, because exact elimination solves each model in about 0.19 s, below the wall-cost bound. The QPU energies come from the recap capture of 12 models. Both K4,4 cells are Closed, class 1 (classical beats cheaply), profile classical cheap and CPU competitive. CPU beats CUDA in both.

| Cell | Best classical | Rank | Energy gap | Strongest attack |
| --- | --- | --- | --- | --- |
| `dcl-k44-l5` | CPU 0.3x (0.1x) | QPU (0.046 s) < CPU (0.083 s) < CUDA (1.42 s) | 0 (3) energy units, QPU 6 / tie 80 / CUDA 14, 22.7 s, 100 models | Exact optimum on 24 of 24 by elimination, at most 0.193 s per model (twentieth fastest) |
| `dcl-k44-l7` | CPU 2.3x (0.7x) | QPU (0.046 s) < CPU (0.668 s) < CUDA (1.42 s) | 0 (2) energy units, QPU 8 / tie 83 / CUDA 9, 26.3 s, 100 models | Exact optimum on 24 of 24 by elimination, 0.19 s |
| `dcl-k88` | CPU 15x (4.3x) | QPU (0.046 s) < CPU (4.37 s) < CUDA (12.4 s) | 0 (1) energy unit, QPU 12 / tie 87 / CUDA 1, 22.2 s, 100 models | not measured |

- Closed by (K4,4): exact elimination, using the planar K4,4 cell-lattice weakness noted in arXiv 1711.01368. The cells close on the wall-cost bound.
  - `dcl-k44-l5`. The exact bucket elimination in Rust certifies the ground state of every model in a median 0.037 s on 6 threads and 0.07 s on 1 thread (11 / 1 / 0 of 12, median margin `-7` energy units, deepest `-13`). The secret-free single-flip anneal arm loses all 12 models at every depth from 2 s to 20 minutes (median margin `+51` at 2 s down to `+19` at 1,200 s).
  - `dcl-k44-l7`. The tree-decomposition solver certifies the ground state of every model in a median 0.526 s on 1 thread, 9 wins and 3 ties against the QPU (median margin `-5`, deepest `-9`). The anneal arm needs about 100 s to tie the QPU on 3 of 12 models and never matches a majority. The 120 s rung matches 4 of 12 models.
- Single-flip GPU on 100 models: `dcl-k44-l5` shows 4 / 65 / 31 (wins / ties / losses against the QPU target) at 32,768 sweeps and 14 / 80 / 6 at 524,288 sweeps (22.7 s, reliable). `dcl-k44-l7` shows 5 / 47 / 48 and 9 / 83 / 8 (26.3 s, reliable). On the 12 production models every GPU rung up to 2,097,152 sweeps loses all models (median margins `+17` to `+32`). GPU parallel tempering with 24 replicas also loses all 12 models, from 16,384 to 4,194,304 sweeps.
- Median energies after postprocess over 12 models, in energy units. `l5` has QPU `-2,112`, CUDA `-2,085`, CPU `-2,114`. `l7`: QPU `-1,820`, CUDA `-1,800`, CPU `-1,821`. `dcl-k88`: all three `-695`.
- `dcl-k88`. The CPU 120 s rung matches 12 of 12 models (4 / 8 / 0, median margin 0, deepest `-3`, 176 s wall, 1,055 core-seconds). The 2 s 100-model rung matches 25 of 30 (4.37 s). Single-flip GPU on 12 models first reaches a majority at 16,384 sweeps (1 win and 6 ties of 12, 0.753 card-seconds). On 100 models at 262,144 sweeps it shows 1 / 80 / 19 (12.4 s), and at 524,288 sweeps 1 / 87 / 12 (22.2 s). The QPU capture of the 100 models ran on 2026-10-27 (100 calls, 4.61 s of access).
- `dcl-k88` QPU vs best classical, class, and profile: not measured. CPU vs CUDA: not measured. No new-pow verdict exists for the family.
- The single-flip GPU comparison for the K4,4 cells at 131,072 sweeps is not measured, because no capture exists.

## Limits and tiling
- Tileable: no for the K4,4 cells. Condition 1 fails. The 12 saved nonce graphs have 150 to 162 independent active components, so the Hamiltonian already separates. Condition 2 is unknown. Condition 3 holds with a cut bound B of 57.6 to 64 units (`l5`) and 50.304 to 54.88 units (`l7`), from 160 cut couplers (128 at the intra-cell strength and 32 at 1). Target attainability is not measured.
- Tileable: unknown for `dcl-k88`. The tiled-generator experiment has not run on the construction of 2026-10-27.
- Chaining: no for the K4,4 cells. Exact elimination solves each link in 0.19 s. For `dcl-k88` chaining is not measured.
- Limit of the approach: the planar cell lattice has low elimination width, so exact elimination solves it. Exact elimination stops on four K8,8 models whose width is 31. The CPU anneal run at 1,200 s covers 1 model of each K4,4 cell and does not reach the QPU target (margin `+19` for `l5`, `+5` for `l7`).
- The call-to-call spread of the QPU is not measured, because no repeat calls exist, and no deeper QPU capture exists.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories: `qpu-suite/work/cells/dcl-k44-l5`, `qpu-suite/work/cells/dcl-k44-l7`, `new-pow/dcl-k44-l5`, `new-pow/dcl-k44-l7`, `new-pow/dcl-k88`, `new-pow/gpu/measurements`, `new-pow/qpu-recap/qpu`, `new-pow/qpu-recap/work/cells`, `new-pow/tiled-generator/dcl-k88`.
