<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Weak-strong cluster networks (F7)

A weak-strong cluster network builds eight-spin ferromagnetic clusters on Chimera sublattice cells. Weak clusters carry a small field and strong clusters carry a full-strength field. The family needs a solver that accepts fields. It was kept as a calibration cell, because the literature closes it by super-spin contraction and parallel tempering with isoenergetic cluster moves (arXiv 1604.01746).

## Construction
The cells follow the LANL `dwig` `wscn` layout on Chimera sublattice cells. The layout offset fixes the cell geometry as (4, 0, 0, 0, 0), so the nonce changes only the bundle signs.

- The strong layer is 133 cells of eight spins, with a field of `-1` (`-1000` milli) on every spin.
- The weak layer is 111 cells of eight spins, with a field of `+0.440` (`+440` milli) on every spin.
- Every coupler inside a cell and every coupler between a weak cell and its strong partner is `-1` (`-1000` milli).
- The cells form 111 weak-strong pairs, each with four pair couplers, and 22 standalone strong cells.
- Strong cells share 77 bundles of four couplers each. The nonce draws one sign per bundle from {`-1`, `+1`}. In the first nonce, 30 bundles are `+1` and 47 are `-1`.
- Inside a cell, one 4-cycle of couplers holds every coupler at `-1`.

The first model has 4,575 spins, of which 1,952 are active (888 at `+0.440` and 1,064 at `-1`), and 4,656 couplers. It uses 42.7 percent of qubits and 11.2 percent of couplers. Topology use is 14.3 percent (6,608 of 46,049).

## Compact representations
- Problem: 256 bits (the nonce, plus the generator version). It describes eight-spin clusters at `-1` inside, weak clusters with fields `+0.440`, strong clusters with fields `-1`, weak-strong pairs at `-1`, and strong-strong bundles of `+-1`. The explicit form of the largest model is 4,656 couplers at 1 bit each (4,656 bits), because 2 values occur.
- Solution: super-spin contraction to one spin per cluster, with an exact ground energy E0 on every model. The compact solution size is not measured. The raw form is 1,952 bits, one per used qubit. The exact solver certifies a best value of `-5,103` energy units on the first model, 9 units below the QPU best of `-5,094`.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `wsc` | Fields `+0.440` and `-1`, J `+-1` | [`corpora/wsc.jsonl`](../../corpora/wsc.jsonl) | Closed | 2 | none |

## Generator and postprocess
Generator: not imported. The research script is `work/cells/wsc/generate.py` in the `qpu-suite` research directory, and it follows the LANL `dwig` `wscn` construction. No structured generator exists under `tools/structured` for this family, and no source file is owned by this family under `sources/`.

Postprocess: the table values apply the `constraints` and `gadget-repair` postprocess for 0.05 s of one core per call. The code is not imported.

## Results
The suite rejected the cell without a capture, because exact elimination solves each model in about 0.06 s, which is below both production cost bounds. The QPU production capture on the 12 recap models costs 0.046 s per call. The cell is Closed, class 1 (classical beats cheaply), profile classical cheap and CPU competitive.

- QPU vs best classical attack: exact ground states give classical ties or wins on 24 of 24 models in at most 0.061 s per model (20 of 24 models).
- Best classical: CPU 4.6x (0.5x). CPU beats CUDA.
- Rank: QPU (0.046 s) < CPU (0.444 s) < CUDA (>24.5 s).
- Energy gap: `-1` (1) energy unit, QPU 59 / tie 41 / CUDA 0, 3.27 s, 100 models. The deciding CUDA run is single-flip at 65,536 sweeps.
- Closed by exact elimination by super-spin contraction, under both production cost bounds. The tree-decomposition solver certifies the best value of every model in a median 0.537 s on one thread, 12 wins of 12 (median margin `-11` energy units, deepest `-14`). The secret-free anneal arm, without exact elimination, wins 7 of 12 models at about 120 s per model.
- Single-flip GPU: no rung from 8,192 to 2,097,152 sweeps reaches the target on a majority of models. On 100 models the ladder shows 0 wins and 41 to 47 ties at 8,192 to 524,288 sweeps, with a median margin of `+1`. On the 12 production models it wins 0 of 12 at every rung (median margin `+24` to `+32`). GPU parallel tempering with 24 replicas wins 0 of 12 at 262,144 sweeps (2.38 s per model, median `+13`) and at 4,194,304 sweeps (33.8 s, median `+10`).
- CPU anneal ladder: the 120 s rung wins 7 of 12 (median `-4`), and the 1,200 s rung covers 1 of 12 models (`-3`).
- Median energies after postprocess over 12 models, in energy units are QPU `-5,084`, CPU `-5,084`, CUDA `-5,064`.
- Deeper QPU anneals: not measured, because no deeper capture exists. No new-pow verdict exists.

## Limits and tiling
- Tileable: no. Condition 1 fails. The 12 saved nonce graphs have 56 active connected components before any cut, so the Hamiltonian already separates. Condition 2 is unknown. Condition 3 holds with a cut bound B of 128 units, but target attainability is not measured.
- Chaining: no. Exact elimination solves each link in 0.06 s.
- The cell is a calibration cell. Super-spin contraction leaves a small graph, so exact elimination solves it and the cell holds no secret beyond the Hamiltonian.
- The call-to-call spread of the QPU is not measured, because no repeat calls exist.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories: `qpu-suite/work/cells/wsc`, `new-pow/wsc`, `new-pow/gpu/measurements/wsc`, `new-pow/qpu-recap/qpu/wsc`, `new-pow/qpu-recap/work/cells/wsc`.
