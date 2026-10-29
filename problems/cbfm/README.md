<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Corrupted biased ferromagnet (F17)

The corrupted biased ferromagnet draws every coupler and every field independently from {0, `-1`, `+1`}, with fixed biases. It has no strong layer and no weak layer. It runs on the Pegasus-density and Zephyr-density chip graphs and needs a solver that accepts fields. It was screened because a field-aligned state looks like a cheap seed, but the corrupted optimum has no closed form.

## Construction
Couplers come from {0, `-1000`, `+1000`} milli (that is, {0, `-1`, `+1`} in units of the full-strength coupler). Fields come from the same set. The class probabilities were re-searched for degree 20, following arXiv 2210.04291 and arXiv 2411.16431.

All present couplers have the same size. The generator omits part of the native couplers. The nonce fixes each coupler, each sign, and each field.

| Cell | Couplers | Fields | First nonce |
| --- | --- | --- | --- |
| `cbfm-p` | Magnitude 1. About half of the native couplers are present. About 10 percent are ferromagnetic and the rest are antiferromagnetic. | Value `-1` on about half of the spins. | 20,672 couplers at `+-1` and 2,274 spins with a field of `+-1` |
| `cbfm-p-f17` | Each of the 41,474 couplers is 0 with probability 0.35, `-1` with 0.10, and `+1` with 0.55. | 0 with probability 0.15, `-1` with 0.85, `+1` with 0. | 22,600 couplers at `+1`, 4,241 at `-1`, 14,633 absent. 3,884 qubits with a field of `-1` and 691 with none. Mean degree 11.8. |
| `cbfm-z` | The same probabilities as the Pegasus model, with the density rescaled so that the mean nonzero degree matches Pegasus: 0 with probability 0.46, `-1` with 0.083, `+1` with 0.457. | 0 with probability 0.15, `-1` with 0.85. | 22,170 couplers (18,682 at `+1` and 3,488 at `-1`). 3,884 qubits with a field of `-1` and 691 with none. |
| `cbfm-h2` | The `cbfm-p` glass with every present coupler at `+-0.500` (`+-500` milli). The fields are the strong terms, because each field of 1 costs twice a coupler. | 0 and `+-1`. | 26,841 couplers at `+-0.500` and 3,884 spins with a field |

Every energy step in `cbfm-p` is a multiple of 2 units, so a single-flip anneal has no strong block to break. In `cbfm-h2` most spins follow their field, so a field-aligned state is a cheap seed and the glass part decides the rest.

Topology use over 12 models:

| Cell | Qubits | Couplers | Combined |
| --- | --- | --- | --- |
| `cbfm-p` | 100 percent (4,575 of 4,575) | 50.5 percent (20,958 of 41,474) | 55.4 percent (25,533 of 46,049) |
| `cbfm-h2` | 100 percent | 65.5 percent (27,151 of 41,474) | 68.9 percent (31,726 of 46,049) |
| `cbfm-p-f17` | 100 percent | 65.5 percent (27,151 of 41,474) | 68.9 percent (31,726 of 46,049) |
| `cbfm-z` | 100 percent | 54.5 percent (22,615 of 41,474) | 59 percent (27,190 of 46,049) |

## Compact representations
- Problem: 256 bits (the nonce, plus the generator version). The couplers and fields are independent draws from {0, `+-1`} with set biases. The explicit form of the largest model is one bit per coupler because 2 values occur: 20,958 bits (`cbfm-p`), 27,151 bits (`cbfm-h2`, `cbfm-p-f17`), and 22,615 bits (`cbfm-z`).
- Solution: none. The field-aligned state is a seed only. The compact solution size is not measured. The raw form is 4,575 bits, one per used qubit.

Worked example for `cbfm-p-f17` (nonce `06fef0b7...`): qubit 0 has a field of `-1` and 7 couplers, one at `-1` to qubit 1 and six at `+1` to qubits 12, 2,298, 2,345, 2,369, 2,415 and 2,462. Qubit 2,345 has no field.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `cbfm-p` | Pegasus density, suite cell. h `-1`, J `+-1`. | [`corpora/cbfm-p.jsonl`](../../corpora/cbfm-p.jsonl) | Closed | 4 | Tier 1 |
| `cbfm-h2` | Couplers at `+-0.500`, fields at 1. | [`corpora/cbfm-h2.jsonl`](../../corpora/cbfm-h2.jsonl) | Closed | 4 | Tier 1 |
| `cbfm-p-f17` | Pegasus density, f17 run. h `-1`, J `+-1`. | none (no frozen model) | Closed | 4 | Tier 2 |
| `cbfm-z` | Zephyr density. h `-1`, J `+-1`. | [`corpora/cbfm-z.jsonl`](../../corpora/cbfm-z.jsonl) | Closed | 4 | Tier 1 |

## Generator and postprocess
- Generators: not imported. The `cbfm-p` generator is `work/cells/cbfm-p/generate.py` and `work/cbfm.py` in the `qpu-suite` research directory. The `cbfm-z` generator is `generate.py` in the `qpu-f17` research directory. It seeds a random generator with `sha256(nonce)` and draws every coupler of the Zephyr graph independently. No structured generator exists under `tools/structured` for this family, and no source file is owned by this family under `sources/`.
- Postprocess: the table values apply the `glasses` and `pt` postprocess. It runs for 0.25 s of one core per call for `cbfm-p` and `cbfm-z`, and for 0.76 s of one core per call for `cbfm-h2` and `cbfm-p-f17`. The code is not imported.

## Results
All four cells are Closed, class 1 (classical beats cheaply). The QPU call is 48 reads at 80 us and costs 0.046 s of access. Both CUDA and CPU beat the QPU in every cell.

| Cell | Best classical | Rank | Energy gap | Single-flip anneal wins (new-pow) |
| --- | --- | --- | --- | --- |
| `cbfm-p` | CUDA 1.5x (0.4x) | QPU (0.046 s) < CUDA (0.435 s) < CPU (1.86 s) | `+18` (12), QPU 0 / tie 1 / CUDA 29, 0.852 s, 30 models | 9 / 0 / 3 of 12, population anneal, 2,560 sweeps, 0.055 s wall |
| `cbfm-h2` | CUDA 0.6x (0.3x) | QPU (0.046 s) < CUDA (0.498 s) < CPU (1.79 s) | `+10` (10), QPU 1 / tie 1 / CUDA 28, 0.948 s, 30 models | 12 / 0 / 0 of 12, population anneal, 2,560 sweeps, 0.062 s wall |
| `cbfm-p-f17` | CUDA 0.6x (0.3x) | QPU (0.046 s) < CUDA (0.487 s) < CPU (1.54 s) | `+25` (19), QPU 2 / tie 1 / CPU 27, 112 s, 30 models | 6 / 1 / 5 of 12, population anneal, 2,560 sweeps, 0.059 s wall |
| `cbfm-z` | CUDA 1.6x (0.5x) | QPU (0.046 s) < CUDA (0.463 s) < CPU (1.41 s) | `+17` (16), QPU 4 / tie 0 / CPU 26, 147 s, 30 models | 7 / 0 / 5 of 12, population anneal, 2,560 sweeps, 0.053 s wall |

Energy gaps are in energy units. The new-pow counts are in the form classical wins / ties / QPU wins.

- Closed by (`cbfm-p`): exact CPU blocks and the single-flip GPU at 8,192 sweeps. A 48-model calibration over 12 class distributions found no distribution that holds. Closed by (`cbfm-h2`, `cbfm-p-f17`, `cbfm-z`): not recorded.
- Strongest attack (`cbfm-p`): GPU single-flip at 8,192 sweeps, 12 / 0 / 0, 5.5 s per model. CPU exact blocks at 8,192 sweeps reject every captured setting. Against the single-flip GPU at 131,072 sweeps, the production call wins 0 of 12.
- The single-flip GPU ladder at 8,192 sweeps on the idle card gives 23 / 1 of 30 models (`cbfm-p`), 26 / 0 of 30 (`cbfm-h2`), 22 / 1 of 30 (`cbfm-p-f17`), and 22 / 0 of 30 (`cbfm-z`). Each wins 12 of 12 at the verdict rung. At 16,384 sweeps the ladder reaches 29 / 1, 28 / 1, 25 / 2, and 25 / 3 of 30 models respectively.
- Median energies after postprocess over 12 models, in energy units. `cbfm-p` has QPU `-10,623`, CUDA `-10,629`, CPU `-10,650`. `cbfm-h2`: QPU `-6,717`, CUDA `-6,720`, CPU `-6,727`. `cbfm-p-f17`: QPU `-12,837`, CUDA `-12,853`, CPU `-12,880`. `cbfm-z`: QPU `-11,363`, CUDA `-11,369`, CPU `-11,388`.
- Deeper QPU anneals (48 reads each, 12 models) reach the production target or lower on more models, but classical still leads. Models below the production target: `cbfm-p` 8 / 10 / 11 of 12 at 200 / 2,000 / 500 us. `cbfm-h2` 7 / 10 / 8. `cbfm-p-f17` 8 / 11 / 9. `cbfm-z` 4 / 8 / 7. The median improvement at 2,000 us is `-8`, `-4`, `-11`, and `-7` energy units.
- CPU depth: the 120 s rung reaches a median of `-44` energy units below the QPU target on 12 of 12 models for `cbfm-p`, `-28` for `cbfm-h2`, `-56` for `cbfm-p-f17`, and `-44` for `cbfm-z`. The 1,200 s rung covers 1 of 12 models (`-50`, `-40`, `-48`, and `-24`).
- Class: 1 (classical beats cheaply). Profile: classical cheap, CPU competitive.

## Limits and tiling
- Tileable: no for `cbfm-p`. Condition 1 fails: the 12 saved nonce graphs have 1 to 2 independent active components, so the full Hamiltonian already separates. Condition 2 (a paired full-instance and joined-tile energy measurement) is unknown. Condition 3 holds for `cbfm-p`: the reference model has 481 cut couplers, and the cut bound B ranges from 481 to 521 units across 12 nonces. Target attainability is not measured.
- Tileable: unknown for `cbfm-h2`, `cbfm-p-f17`, and `cbfm-z`. All three conditions are unknown. The generators have 3,884 nonzero fields, and the zero-field sign-alignment protocol needs zero local fields so tile signs can align without changing energy. The measurement needed is a zero-field tiled instance and a repeat of the paired experiment.
- Chaining adds nothing. GPU single flips at 8,192 sweeps beat the call per link. CPU anneals do so in 0.09 to 0.26 s on 6 threads. One CPU job needs 0.089 s per link (0.064 s for `cbfm-z`), and 1,000 jobs keep the same time per link.
- Verdict CPU wall by variant: 0.055 s for Pegasus density (9 of 12 models), 0.059 s for the f17 run (7 of 12), 0.053 s for Zephyr density (7 of 12), and 0.062 s for couplers at 0.500 (12 of 12).
- The call-to-call spread of the QPU is not measured, because no repeat calls exist. The CPU 1,200 s rung covers 1 of 12 models.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories: `qpu-suite/work/cells/cbfm-p`, `qpu-suite/qpu/cbfm-p`, `qpu-f17/work/cells/cbfm-h2`, `qpu-f17/qpu/cbfm-h2`, `qpu-f17/work/cells/cbfm-z`, `qpu-f17/qpu/cbfm-z`, `qpu-f17/qpu/cbfm-p`, `new-pow/cbfm-p`, `new-pow/cbfm-h2`, `new-pow/cbfm-p-f17`, `new-pow/cbfm-z`, `new-pow/retime`, `new-pow/gpu/measurements`, `new-pow/tiled-generator/cbfm-h2`.
