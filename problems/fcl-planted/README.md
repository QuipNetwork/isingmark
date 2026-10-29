<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Planted frustrated cluster loops (F4)

Planted frustrated loops start from a hidden planted state and add random loops that are frustrated against it. The planted state is a ground state by construction, and its energy follows from the loop lengths. The family was screened as a problem with a known optimum, which lets a verifier check any answer. The planted state turned out to be recoverable from the public nonce.

## Construction
The planted state is the strong layer. The nonce bytes pass through SHA256 and seed a NumPy stream. The stream draws the planted state first, then the random walks and the antiferromagnetic edge choice of each loop. Each accepted loop has 3 to 12 vertices. It adds `-1` to every edge in the frame of the planted state and `+2` to one chosen edge (R = 1), so one edge frustrates the loop. The generator rejects a whole walk when it pushes any edge sum outside [-1, 1]. The accumulated sums scale to `+-1` (`+-1000` milli), and edges whose sum returns to zero are removed. No separate weak layer exists, because every coupling comes from the loops. All fields are 0. The generator plants floor(alpha x 4,575) loops.

| Cell | Loops | First nonce | Qubits | Couplers |
| --- | --- | --- | --- | --- |
| `fcl-planted-a0.2` | 915 accepted loops (alpha 0.2) | 1,355 walks attempted, 169 triangles, 297 loops of 4 vertices | 69.7 percent (3,223 of 4,575) | 11.5 percent (4,890 of 41,474) |
| `fcl-planted-a0.3` | 1,372 loops (alpha 0.3) | 2,256 walks attempted, 264 triangles, 459 loops of 4, 26 loops of 12. 6,812 couplers on 3,801 active spins. 2,225 vertices lie in more than one loop. | 84.8 percent (3,881 of 4,575) | 16.8 percent (7,002 of 41,474) |

Topology use is 17.6 percent (`a0.2`) and 23.6 percent (`a0.3`).

## Compact representations
- Problem: 256 bits (the nonce, plus the generator version). It describes the sum over loops of frustrated loops planted at s*. The count is alpha x 4,575 loops of 3 to 12 spins, with edge sums in [-1, 1] scaled to `+-1`. The explicit form of the largest model is one bit per coupler: 4,890 bits (`a0.2`) and 7,002 bits (`a0.3`).
- Solution: E0 = `-1` x sum over loops of (l - 2), where l is the loop length. The planted state s* and its gauge orbit reach E0. The compact solution size is not measured. The raw form is 3,223 bits (`a0.2`) and 3,881 bits (`a0.3`), one per used qubit.

Worked examples (nonce `e9717485...`): nonce replay gives the planted state of `a0.2` at `-3,060` energy units, equal to the QPU best. For `a0.3` the planted state gives `-4,464`, which is 4 units below the QPU best of `-4,460`.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `fcl-planted-a0.2` | alpha 0.2, J `+-1` | [`corpora/fcl-planted-a0.2.jsonl`](../../corpora/fcl-planted-a0.2.jsonl) | Closed | 4 | none |
| `fcl-planted-a0.3` | alpha 0.3, J `+-1` | [`corpora/fcl-planted-a0.3.jsonl`](../../corpora/fcl-planted-a0.3.jsonl) | Closed | 2 | none |

## Generator and postprocess
- Generator: [`fcl_planted.py`](../../tools/structured/isingmark_structured/families/fcl_planted.py). It ports the suite generator `work/fcl.py`. Random non-backtracking walks close simple cycles of 3 to 12 spins. Each cycle adds `-1` to its bonds and `+1` to one random bond. A cycle that would push any bond outside [-1, 1] is rejected whole. The generator plants floor(alpha x spins) loops, then a random planted state gauges the result. Only couplers with a nonzero sum appear. Cell parameters: [`params/fcl-planted-a0.2.json`](params/fcl-planted-a0.2.json) and [`params/fcl-planted-a0.3.json`](params/fcl-planted-a0.3.json).
- The attack that closed the family replays the public generator from the nonce. The research script for the coefficient-only arm is `work/fcl_attack.py` in the `qpu-suite` research directory, and it is not imported.
- Postprocess: the table values apply the `glasses` and `pt` postprocess for 0.25 s of one core per call. The code is not imported. No source file is owned by this family under `sources/`.

## Results
The suite rejected both cells without a capture, because the planted state is recoverable from the public nonce in under 1 ms. The QPU values come from the recap capture of 12 models, 48 reads at 80 us, 0.046 s per call. Both cells are Closed, class 1 (classical beats cheaply), profile classical cheap and CPU competitive. CPU beats CUDA in both.

| Cell | Best classical | Rank | Energy gap | Strongest attack |
| --- | --- | --- | --- | --- |
| `fcl-planted-a0.2` | CPU 0.5x (0.1x) | QPU (0.046 s) < CPU (0.136 s) < CUDA (0.388 s) | not measured | 24 / 0 / 0 against the planted optimum, public-nonce recovery, under 1 ms (no capture) |
| `fcl-planted-a0.3` | CPU 0.1x (0.0x) | CPU (0.042 s) = QPU (0.046 s) < CUDA (0.412 s) | 0 (1) energy unit, QPU 0 / tie 27 / CPU 3, 107 s, 30 models | 24 / 0 / 0 against the planted optimum, public-nonce recovery, under 1 ms (no capture) |

- Closed by: planting leakage. Rerunning the public generator recovers s* and the planted energy on 24 of 24 models in under 1 ms per model. Any planted family is recoverable from a public nonce. Coefficient-only attacks are reported separately.
- Nonce replay on 12 models (wins / ties / losses against the QPU target): `a0.2` reaches the planted ground in a median 0.131 s, 2 / 10 / 0. `a0.3` does so in a median 0.051 s on one thread, 9 / 3 / 0 (median margin `-4` energy units, deepest `-16`).
- Secret-free arms read only the Hamiltonian. For `a0.2`, a multi-spin anneal of 256 sweeps gives the same energies in a median 0.112 s (2 / 10 / 0). For `a0.3`, the multi-spin anneal reaches the planted ground on all 12 models by 8 s and matches a majority at about 0.71 s. Its wins / ties / losses are 0 / 0 / 12 at 128 and 256 sweeps, 3 / 4 / 5 at 512, 5 / 4 / 3 at 1,024, 6 / 3 / 3 at 2,048, and 8 / 4 / 0 at 4,096 sweeps (1.29 s).
- Single-flip GPU at 8,192 sweeps on the idle card: `a0.2` shows 0 wins and 30 ties of 30 models (0.388 card-seconds) and 2 / 10 / 0 on the 12 production models (0.342 card-seconds). `a0.3` shows 2 / 26 / 2 of 30 (0.412 card-seconds) and 9 / 3 / 0 on the 12 production models.
- CPU depth: every rung from 2 s to 1,200 s matches the target on all 12 models for `a0.2` (margin 0). For `a0.3`, the 120 s rung reaches a median `-4` energy units below the target on 12 of 12 models.
- Median energies after postprocess over 12 models, in energy units: QPU, CPU, and CUDA all `-3,105` (`a0.2`) and `-4,552` (`a0.3`).
- The single-flip GPU comparison at 131,072 sweeps is not measured, because no capture exists. No new-pow verdict exists for the family.

## Limits and tiling
- Tileable: no, for both cells. Condition 1 fails: the 12 saved nonce graphs have 19 to 38 (`a0.2`) and 3 to 12 (`a0.3`) active connected components before any cut, so the Hamiltonian already separates. Condition 2 is unknown. Condition 3 holds with a cut bound B of 92 to 126 units (`a0.2`, 114 cut couplers in the reference model) and 142 to 178 units (`a0.3`, 162 cut couplers). Target attainability is not measured.
- Chaining: no. Public-nonce recovery gives E0 in under 1 ms per link, so a chain closes link by link.
- Limit of the approach: a planted optimum depends on a secret that the public nonce reveals. A verifier can check an answer, but a miner can replay the generator instead of solving the problem.
- The call-to-call spread of the QPU is not measured, because no repeat calls exist, and no deeper QPU capture exists. The CPU 1,200 s rung covers 1 of 12 models.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories: `qpu-suite/work/cells/fcl-planted-a0.2`, `qpu-suite/work/cells/fcl-planted-a0.3`, `new-pow/fcl-planted-a0.2`, `new-pow/fcl-planted-a0.3`, `new-pow/gpu/measurements/fcl-planted-a0.2`, `new-pow/gpu/measurements/fcl-planted-a0.3`, `new-pow/qpu-recap/qpu/fcl-planted-a0.2`, `new-pow/qpu-recap/qpu/fcl-planted-a0.3`.
