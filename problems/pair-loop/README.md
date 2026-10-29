<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Strong pairs joined in frustrated loops (F3)

Strong pair-loop problems tie spins into ferromagnetic pairs at the strongest coupling and then join the pairs into frustrated loops at half strength. A weak background sits on every other coupler. Contracting each pair to one spin leaves a reduced glass that carries the loops. The family was built as a calibration of the per-level cost. Contraction plus cycle moves was expected to close it, and both attacks did.

## Construction
The strong layer is a greedy matching of ferromagnetic pairs at `-1` (`-1000` milli, in units of the full-strength coupler), joined into frustrated loops by 0.500 couplers (`±500` milli). Each loop chains 6 to 10 pairs, and its signs leave an odd number of antiferromagnetic bonds, so each loop stays frustrated. Every other coupler is weak, at `+0.060` or `-0.060` (`±60` milli). Fields are 0. The nonce alone fixes the structure. The first model has 2,239 pairs at `-1`, 2,108 loop couplers at `±0.500` in 268 loops, and 37,127 weak couplers at `±0.060`.

Topology use: 100% of qubits, 100% of couplers, 100% of the topology.

## Compact representations
- Problem: 256 bits (nonce, plus the generator version). Strong bonds of 0.800 to 1 on matchings or loops over a weak background.
- Solution: 4,575 bits raw, one per used qubit. The compact form is not measured. The strong layer fixes s_a s_b = −sign(J), a 2^pairs sector, and contraction gives the reduced glass.
- Compact formula: partly. No closed form exists for the reduced glass.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `pair-loop` | pairs at `-1`, loops of 6 to 10 pairs at `±0.500`, background `±0.060` | [`corpora/pair-loop.jsonl`](../../corpora/pair-loop.jsonl) | Closed | 3 (suite tier 3) | Tier 1 |

## Generator and postprocess
The generator and the postprocess are not imported. No source file in `sources/` belongs to this family. The research generator is `generate.py` in the research cell directory `qpu-suite/work/cells/pair-loop`. The production postprocess is `dimers` plus `csa`, 0.25 s of one core per call, with a median gain of 1 (0) energy units. The attacks use pair contraction and cycle moves.

## Results
- QPU: no capture.
- Strongest attack: both CPU attacks (pair contraction and cycle moves) beat the deepest GPU rung on 12 of 12 screened models.
- Rank: QPU (0.046 s) < CPU (1.94 s) < CUDA (>75.9 s). CPU beats CUDA.
- Best classical: CPU 6.5× (1.9×). CUDA gap: >256× (>75×).
- Energy gap: `+5` (2), QPU 0 / tie 0 / CPU 30, 37.6 s, 30 models.
- GSE medians in energy units are QPU `-3,426`, CUDA `-3,424`, CPU `-3,434`.
- Deep sweep: CPU 3 (2) / 6,206 sweeps / 2 s.
- New-pow adversary: 8 / 1 / 3 of 12, compsa at 160 sweeps, 0.042 s wall, ratio vs access 0.92.
- Run counts (QPU / CUDA / CPU): 12 / 174 / 474.
- Seed spread: the CPU verdict kernel matches the QPU target on 26 of 36 model-runs (9 of 12, 8 of 12 and 9 of 12 over three runs).
- GPU: single-flip rungs lose to the QPU target through 524,288 sweeps (median `+6` energy units). At 2,097,152 sweeps it wins 3, ties 1 and loses 8 of 12. Parallel tempering at 4,194,304 sweeps loses all 12 (median `+11`).
- Closed by: classical calibration complete. Capture omitted under the Tier 3 rule.

## Limits and tiling
Tileable: yes. All three tiled-generator conditions pass across 12 nonces. The largest absolute energy delta is 1.15 units per tile, with tiled wins / ties / direct wins of 10 / 0 / 2. Seam strength is 0.060 and the total satisfied-seam benefit is 0.240 units. Chaining adds nothing. Pair moves or plain anneals beat the call per link at 0.08 to 6 s, so a chain only multiplies the classical lead.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories: `regimes/qpu-suite/work/cells/pair-loop`, `regimes/new-pow/pair-loop`, `regimes/new-pow/tiled-generator/pair-loop`.
