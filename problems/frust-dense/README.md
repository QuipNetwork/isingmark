<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Dense frustrated blocks (F8)

Dense frustrated blocks fill the chip with disjoint blocks of 12 or 16 connected spins. Every coupler inside a block is a strong random-sign bond. A weak random-sign background joins the blocks. The answer is close to a product of block optima. Each block optimum is an enumeration of 2^k states. The family fails the overlap property because the blocks are disjoint. Both cells are demoted, and the classical side closes them.

## Construction
Connected blocks of k spins (k = 12 or 16) carry every internal coupler at `±1` (`±1000` milli, random sign, in units of the full-strength coupler). The background couplers carry `±0.125` (`±125` milli) and run between blocks, so the background acts on each block only as a field. Fields are 0. Blocks are disjoint, and the block boundaries come from the research script `dense_blocks.py`.

- `frust-dense-k12`: the first model has 350 blocks of 12 spins, 11,888 strong couplers and 29,586 background couplers.
- `frust-dense-k16`: the first model has 267 blocks of 16 spins (4,272 spins), 303 single spins with no strong coupler, 15,036 strong couplers and 26,438 background couplers.

Topology use: 100% of qubits, 100% of couplers, 100% of the topology (both cells).

## Compact representations
- Problem: 256 bits (nonce, plus the generator version). Disjoint 12- or 16-spin dense blocks with a `±0.125` background.
- Solution: 4,575 bits raw, one per used qubit. The compact form is not measured. The answer is close to a product of block optima, each one a 2^k enumeration.
- Compact formula: partly. Disjoint dense blocks of 12 or 16 random-sign bonds at 1 plus a 0.125 background. The answer is near a product of block optima.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `frust-dense-k12` | 12-spin blocks, 0.125 background | [`corpora/frust-dense-k12.jsonl`](../../corpora/frust-dense-k12.jsonl) | Closed | 4 (suite tier 3) | Tier 1 |
| `frust-dense-k16` | 16-spin blocks, 0.125 background | [`corpora/frust-dense-k16.jsonl`](../../corpora/frust-dense-k16.jsonl) | Closed | 3 (suite tier 3) | Tier 1 |

## Generator and postprocess
The generators are not imported. No source file in `sources/` belongs to this family. The research generators are `generate.py` in the research cell directories `qpu-suite/work/cells/frust-dense-k12` and `qpu-suite/work/cells/frust-dense-k16`. Both use the block builder `qpu-suite/work/dense_blocks.py`. The production postprocess is `glasses` plus `block_sa`, 0.76 s of one core per call for k12 and 0.25 s for k16. It lifts the QPU median from `-7,068` to `-7,104` (k12) and from `-7,870` to `-7,903` (k16) energy units.

## Results
Neither cell has a suite-run QPU capture, because the Tier 3 rule omitted it. The new-pow recap holds 12 production calls for each cell, and the QPU energies below come from those calls. Both closed with the classical calibration complete.

| Measure | `frust-dense-k12` | `frust-dense-k16` |
| --- | --- | --- |
| Strongest attack | CPU at 128 sweeps beats the GPU at 32,768 sweeps on 12 of 12 | same |
| Rank | QPU (0.046 s) < CPU (2.13 s) < CUDA (>31.1 s) | QPU (0.046 s) < CPU (1.91 s) < CUDA (>65.8 s) |
| Best classical (CPU gap) | CPU 2.6× (1.4×) | CPU 6.4× (1.9×) |
| CUDA gap | >39× (>20×) | >222× (>65×) |
| Energy gap | `+11` (4), QPU 0 / tie 0 / CPU 30, 8.90 s, 30 models | `+14` (3), QPU 0 / tie 0 / CPU 30, 1.91 s, 30 models |
| Deep sweep | CPU 8 (4) / 4,412 sweeps / 2 s | CPU 14 (3) / 4,174 sweeps / 2 s |
| GSE medians (QPU / CUDA / CPU) | `-7,068` / `-7,091` / `-7,120` | `-7,870` / `-7,895` / `-7,924` |
| New-pow wins | 12 / 0 / 0 of 12, compsa, 64 sweeps | 12 / 0 / 0 of 12, compsa, 64 sweeps |
| New-pow wall, ratio vs access | 0.017 s, 0.37 | 0.024 s, 0.526 |
| Runs (QPU / CUDA / CPU) | 12 / 154 / 338 | 12 / 309 / 299 |

CPU beats CUDA on both cells. Both cells closed with the capture omitted at the stop. A CPU run at 1,200 s reaches a median of `-44` (k12) and `-51` (k16) energy units below the QPU target on 1 model.

GPU detail: for k12 the single-flip ladder first reaches a majority at 524,288 sweeps (10 of 12 models, 38.6 card-s) and wins 12 of 12 from 1,048,576 sweeps (138 card-s). For k16 it first reaches a majority at 131,072 sweeps (7 of 12, 9.10 card-s). GPU parallel tempering at 262,144 sweeps with 48 replicas wins 10 of 12 (7.47 s per model).

## Limits and tiling
Tileable: no for both cells. Condition 1 fails under an explicit dense-block exclusion, because the full graph stays coupled by `±0.125` bonds and prevents an exact product. Condition 2 (paired energy measurement) is unknown. Chaining adds nothing. CPU at 128 sweeps beats the deepest GPU rung per link.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories: `qpu-suite/work/cells/frust-dense-k12`, `qpu-suite/work/cells/frust-dense-k16`, `new-pow/frust-dense-k12`, `new-pow/frust-dense-k16`, `qpu-suite/work/dense_blocks.py`.
