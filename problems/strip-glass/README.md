<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Strip glass (F19)

Strip glass puts a strong random-sign layer on one qubit orientation of the chip and a weak layer inside the other orientation. Once the strong orientation is fixed, the remaining spins form short ladders of 20 to 24 spins. A transfer matrix solves each ladder exactly. The family was screened to test whether a half-chip exact move closes the structure. It does, so the cell is an attack validation, not a hard problem.

## Construction
The strong layer is a set of bonds at `±1` (`±1000` milli, in units of the full-strength coupler) on one qubit orientation only, which covers all external and odd couplers with a random sign. It splits into 221 components of 19 to 24 spins, with about 1,900 frustrated triangles per model. The weak layer is every other (internal) coupler at `±0.125` (`±125` milli). All 33,302 weak couplers run between components, so the weak layer acts only as a field on each component. Fields are 0. The nonce seeds the cell generator, which derives every coupler and its sign. The first nonce gives 8,172 strong couplers and 33,302 weak couplers. The strong layer lives in one qubit orientation, so a half-chip exact move must close it.

Topology use: 100% of qubits, 100% of couplers, 100% of the topology.

## Compact representations
- Problem: 256 bits (nonce, plus the generator version). Strong bonds sit on one qubit orientation only.
- Solution: 4,575 bits raw, one per used qubit. The compact form is not measured. Given the other orientation, an exact ladder transfer matrix gives the answer.
- Compact formula: yes. Strong `±1` bonds sit only on external and odd couplers (one orientation) and 0.125 internal bonds. Given the other orientation, each 20 to 24 spin ladder solves exactly by transfer matrix.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `strip-glass` | 1 strong bonds, 0.125 internal bonds | [`corpora/strip-glass.jsonl`](../../corpora/strip-glass.jsonl) | Closed | 4 (suite tier 3) | Tier 2 |

## Generator and postprocess
The generator and the postprocess are not imported. No source file in `sources/` belongs to this family. The research generator is `generate.py` in the research cell directory `qpu-suite/work/cells/strip-glass`. The production postprocess is `glasses` plus `block_sa`, 0.76 s of one core per call. It lifts the QPU median from `-6,095` to `-6,116` energy units (gain 16 (4) units per model).

## Results
- QPU: no capture. QPU access used is 0 s. The QPU time-to-target (TTS) of 0.046 s is the nominal one-call figure.
- Best classical attack: the CPU half-chip exact move at 8,192 sweeps beats the deepest GPU rung on 12 of 12 models (60.3 s per model).
- Rank: QPU (0.046 s) < CPU (1.85 s) < CUDA (>8.19 s). CPU beats CUDA.
- Best classical: CPU 2.3× (1.2×). CUDA gap: >10× (>5.4×).
- Energy gap: `+11` (5), QPU 0 / tie 0 / CPU 30, 7.68 s, 30 models.
- Ground state energy (GSE) medians in energy units are QPU `-6,095`, QPU after postprocess `-6,116`, CUDA `-6,104`, CPU `-6,132`.
- Deep sweep: CPU 11 (4) / 1,400 sweeps / 2 s.
- GPU: the single-flip ladder first reaches a majority at 131,072 sweeps (10 of 12 models, 9.10 card-s per model). The 30-model run at the same rung wins only 3 of 30.
- New-pow adversary: 11 / 0 / 1 of 12 (wins / ties / losses), component-state annealer (compsa) at 128 sweeps, 0.341 s wall, ratio vs access 7.42.
- Run counts (QPU / CUDA / CPU): 12 / 90 / 372.
- Closed by: classical calibration complete. Capture omitted under the Tier 3 rule.

## Limits and tiling
Tileable: no. Chaining: no. The conditional ladder solve closes each link, so a chain is closed link by link. The tiled-generator run composes 4 hash-chained tiles, and its results sit in this table.

| Item | Result |
| --- | --- |
| Condition 1 and condition 3 | pass |
| Condition 2 (energy agreement) | fails, with a largest absolute delta of 3.75 units per tile |
| Tiled wins / ties / direct wins | 12 / 0 / 0 |
| Tile solvers reaching their targets | 41 of 48 |
| Deepest classical result at 20 minutes | CPU median of `-22` energy units on 1 model |

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories: `regimes/qpu-suite/work/cells/strip-glass`, `regimes/new-pow/strip-glass`, `regimes/new-pow/tiled-generator/strip-glass`.
