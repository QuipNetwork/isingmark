<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Sparse native 3D +-J glass (F15, F16)

These cells are +-J spin glasses on three-dimensional lattices that embed natively in the chip graph. One layout is a diamond lattice with one qubit per site. The other is a cubic layout with the dimer chains removed. They were screened because a native three-dimensional glass has a hard ground state and uses few couplers.

## Construction
Both cells have one layer and no weak layer. The nonce draws the sign of every coupler. The layout is the same for every nonce. All fields are 0.

- `diamond-3d`: the King et al. 2025 diamond-lattice layout. It is 52 by 52 by 16 diamond sites mapped onto the working graph, which leaves 4,574 active spins of 4,575 and 8,346 couplers. Every lattice bond is `+-1` (`+-1000` milli). Every other coupler is 0. Each active spin has degree 4 or less, and 3,455 spins have degree 4. About half of the fundamental cycles are frustrated (1,893 frustrated cycles for the first nonce). The first model has 4,207 couplers at `+1` and 4,139 at `-1`. It uses 99.98 percent of qubits and 20.1 percent of couplers.
- `cubic-no-dimer` is a clipped copy of the King et al. 2025 cubic-dimer layout, placed on the chip at a shift of `-23` by `-24`. The second z coupler of each logical z bond is dropped. Every retained coupler is a random glass bond at `+1` or `-1`, including the 1,052 former pair edges, so no bond is a strong dimer and no chain exists. The layout has 2,295 active spins, 4,118 couplers, and degree 4 or less. The first model has 2,051 couplers at `+1` and 2,067 at `-1`. It uses 50.2 percent of qubits (2,295 of 4,575) and 9.9 percent of couplers (4,118 of 41,474). Topology use is 13.9 percent.

Topology use of `diamond-3d` is 28.1 percent (12,920 of 46,049).

## Compact representations
- Problem: 256 bits (the nonce, plus the generator version). It describes a `+-1` Edwards-Anderson glass on a diamond or cubic layout. The explicit form is one bit per coupler: 8,346 bits (`diamond-3d`) and 4,118 bits (`cubic-no-dimer`).
- Solution: none. The ground state is NP-hard with no closed form. The compact solution size is not measured. The raw form is 4,575 bits (the largest model) and 2,295 bits for `cubic-no-dimer`, one per used qubit.

Worked example for `diamond-3d` (nonce `e9717485...`): spin 91 has degree 4, with couplers `+1` to spin 2309, `-1` to spin 2369, `+1` to spin 2392, and `-1` to spin 2474. Worked example for `cubic-no-dimer`: spin 92 is the center of a five-spin star with bonds `+1` to spin 91, `-1` to spin 93, `+1` to spin 2497 (a former pair edge), and `+1` to spin 2587.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `cubic-no-dimer` | Chain-free cubic 3D glass, J `+-1` | [`corpora/cubic-no-dimer.jsonl`](../../corpora/cubic-no-dimer.jsonl) | Closed | 4 | Tier 1 |
| `diamond-3d` | Diamond-lattice 3D glass, J `+-1` | [`corpora/diamond-3d.jsonl`](../../corpora/diamond-3d.jsonl) | Closed | 3 | Tier 1 |

## Generator and postprocess
- `diamond-3d`: [`diamond_3d.py`](../../tools/structured/isingmark_structured/families/diamond_3d.py). It ports the suite generator `work/diamond.py`. A 52×52×16 diamond lattice goes on the chip at the integer shift that keeps the most live spins, then the most couplers, then the smallest shift. The retained bonds, sorted by spin pair, take a protocol draw from [-1000, 1000].
- `cubic-no-dimer`: [`cubic_no_dimer.py`](../../tools/structured/isingmark_structured/families/cubic_no_dimer.py). It ports the suite generator `work/cubic.py`. A 26×26×8 cubic dimer layout goes on the chip with the same shift rule. Every retained physical coupler, the former dimer bonds included, takes its own protocol draw from [-1000, 1000] in sorted spin-pair order.
- The generators share the `lattices` and `zephyr` modules of the same package.
- Cell parameters: [`params/`](params/).
- Postprocess: the table values apply the `glasses` and `pt` postprocess for 0.76 s of one core per call. The code is not imported. No source file is owned by this family under `sources/`.

## Results
Both cells are Closed, class 1 (classical beats cheaply), profile classical cheap and CPU competitive. The QPU call is 48 reads at 80 us and costs 0.046 s. Both CPU and CUDA beat the QPU in both cells. The closing attack is star updates on the CPU and the single-flip GPU.

| Cell | Best classical | Rank | Energy gap | Strongest attack (classical / tie / QPU) |
| --- | --- | --- | --- | --- |
| `diamond-3d` | CPU 33x (18x) | QPU (0.046 s) < CUDA (>0.297 s) < CPU (26.8 s) | `+4` (7), QPU 7 / tie 5 / CPU 18, 107 s, 30 models | 11 / 0 / 1, GPU single-flip, 131,072 sweeps, 34.4 s (8,192 sweeps: 10 wins of 12, 2.2 s) |
| `cubic-no-dimer` | CUDA 0.5x (0.3x) | QPU (0.046 s) < CUDA (0.382 s) < CPU (2.14 s) | `+5` (3), QPU 0 / tie 1 / CPU 29, 140 s, 30 models | 12 / 0 / 0, GPU single-flip, 131,072 sweeps, 34.8 s (8,192 sweeps: 9 wins of 12, 2.1 s) |

Energy gaps are in energy units.

- Star updates at 8,192 sweeps beat the production call on 11 of 12 models for `diamond-3d` (gap `-13,000`). At 2,000 us the QPU result is 9 / 1 / 2. For `cubic-no-dimer` the production call wins 1 of 12.
- No single-flip GPU rung reaches the target on a majority of `diamond-3d` models on the idle card at 8,192 sweeps over 30 models (3 wins, 1 tie, 26 losses, CUDA bound `>0.297 s`). On the 12 production models the verdict rung wins 10 / 1 of 12 (0.599 card-seconds). The CPU reaches the target at the 30 s 100-model rung (26.8 s).
- For `cubic-no-dimer` the GPU at 8,192 sweeps on 30 models gives 9 wins and 11 ties (0.382 card-seconds). The 32,768-sweep rung over 30 models is reliable, with 22 wins and 7 ties (1.22 s).
- In the new-pow campaign, a population anneal at 3,072 sweeps wins 7 / 1 / 4 of 12 on `diamond-3d` (0.045 s wall). A multi-spin coded anneal at 2,048 sweeps wins 10 / 1 / 1 of 12 on `cubic-no-dimer` (0.046 s wall, 64 replicas per word).
- Median energies after postprocess over 12 models, in energy units. `diamond-3d` has QPU `-6,290`, CUDA `-6,283`, CPU `-6,302`. `cubic-no-dimer` has QPU `-3,137`, CUDA `-3,137`, CPU `-3,143`.
- Deeper QPU anneals (48 reads each) go below the production target on only some models. `diamond-3d`: 7, 9, and 7 of 12 below the production target at 200, 2,000, and 500 us. `cubic-no-dimer`: 7, 6, and 6 of 12.
- CPU depth: the 120 s rung reaches a median of `-25` (`diamond-3d`) and `-11` (`cubic-no-dimer`) energy units below the QPU target on 12 of 12 models. The 1,200 s rung covers 1 of 12 models (`-10` and `-14`).
- The full-cohort GPU crossing is not measured.

## Limits and tiling
- Tileable: no, for both cells.
  - `cubic-no-dimer`. Condition 1 fails. The 12 saved nonce graphs have 14 active connected components before any cut, so the Hamiltonian already separates. Condition 2 is unknown. Condition 3 holds with a cut bound B of 140 units, but target attainability is not measured.
  - `diamond-3d`. Condition 2 fails. The tiled-generator protocol composes 4 hash-chained tiles with a 30.0 s budget per tile. The largest energy delta is 6.5 units per tile, and tiled wins / ties / direct wins are 12 / 0 / 0. Conditions 1 and 3 hold (seam strength 1, B of 4). Tile solvers reached 42 of 48 targets.
- Chaining adds nothing. A single-flip anneal matches the QPU per link at about 0.08 to 0.09 s (tier 1). One CPU job needs 0.091 s per link (`diamond-3d`) and 0.078 s (`cubic-no-dimer`), and 1,000 jobs keep the same time per link.
- The ground state is NP-hard, but the sparse native lattice gives the classical solvers a cheap local structure.
- The call-to-call spread of the QPU is not measured, because no repeat calls exist. The CPU 1,200 s rung covers 1 of 12 models.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories: `qpu-suite/work/cells/cubic-no-dimer`, `qpu-suite/work/cells/diamond-3d`, `qpu-suite/qpu/cubic-no-dimer`, `qpu-suite/qpu/diamond-3d`, `new-pow/cubic-no-dimer`, `new-pow/diamond-3d`, `new-pow/retime/cubic-no-dimer`, `new-pow/retime/diamond-3d`, `new-pow/gpu/measurements/cubic-no-dimer`, `new-pow/gpu/measurements/diamond-3d`, `new-pow/tiled-generator/diamond-3d`.
