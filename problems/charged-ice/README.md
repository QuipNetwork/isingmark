<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Charged ice tiles

This family gives each ice tetrahedron a charge of 0 or plus or minus 2 and sets the charge with local fields. A charged tetrahedron has fewer ground states than the six-state ice rule, and the fields break the gauge degeneracy that protects the plain ice cells. It was screened to see whether fields on the tiles harden the problem. They did not. Classical solvers reach the QPU result in seconds.

## Construction
Couplings are in units of the full-strength coupler (J = 1), with the milli value in a code span. The working graph has 4,575 qubits and 41,474 couplers.

- Strong layer: the suite tetrahedra at 0.250 (`250`), 10,710 couplers per model at ±0.250.
- Charge: each tetrahedron carries charge q of 0, +2, or −2, drawn with probabilities 0.5, 0.25, and 0.25. Fields of 0.500 (`500`) and 1 (`1000`) on 2,438 qubits set the charges.
- Weak layer: 30,764 couplers at ±0.031 (`±31`).
- Distinct values: h {±0.500, ±1}, J {±0.031, ±0.250}. The nonce derives every tile gauge, every field, and every weak sign.
- Topology use: 100 percent of qubits and 100 percent of couplers.

## Compact representations
- Problem (256 bits: the nonce plus the generator version): summed tile terms `A/2 (sum g s - q)^2` over the suite tetrahedra, where each tetrahedron carries a charge q of 0, +2, or −2 set by the fields.
- Solution (4,575 bits raw, compact size not measured): charge-2 tiles keep 4 states (1-in-4).
- Explicit size of the largest of 12 models: 41,474 couplers × 2 bits = 82,948 bits (4 values).

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `ice-charge` | Tiles at `250`, charge 0 or ±2 from fields `500` and `1000`, glass `±31` | [`corpora/ice-charge.jsonl`](../../corpora/ice-charge.jsonl) | Closed | 3 | Tier 2 |

Tier 3 and tier 4 mean that a classical run inside 120 s beats the postprocessed QPU result on at least 60 percent of 30 or more common models. Tier 4 means the best classical run is also within five times the QPU wall time. Tier 1 and tier 2 mean no classical run beats the QPU in that way, and tier 1 means the QPU is also fast. New-pow Tier 1 means classical is strictly lower on a majority of models within 0.1 s. New-pow Tier 2 means parity within 20 min.

## Generator and postprocess
- Generator: [`generate3.py`](sources/qpu-f17/generate3.py) sets the scale to 250, the background to ±31, and draws the charge q from {0, +2, −2}.
- Postprocess: `ice-tiles` ([`icetiles.py`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/icetiles.py), [`icetiles.c`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/icetiles.c), [`method.py`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/method.py)), with `anneal:sa=1,b0=2,len=64,fe=16,nat48=1` at 0.25 s of one core per call. It gains 1 (1) energy unit over greedy descent.

## Results
- Class and profile: not recorded. Tier 3. New-pow Tier 2.
- Energies, median over models, in energy units: QPU −1,526, QPU after postprocess −1,528, CPU −1,533, CUDA −1,530.
- Energy gap: +2 (1), QPU 0 / tie 0 / CUDA 100, 4.56 s, 100 models. The deciding run is a single-flip GPU anneal at 65,536 sweeps.
- Best classical: CPU at 6.4 times the QPU access time (1.9 times the QPU wall time). Rank by time to the postprocessed target: QPU (0.046 s) < CPU (1.90 s) < CUDA (4.56 s). The CUDA cluster anneal does not reach the target below 65,536 sweeps (1 of 12 models matches there).
- New-pow campaign (2026-10-20): 12 / 0 / 0 over 12 models for the single-flip population anneal (`glasssa`) at 8,192 sweeps. Median wall 0.208 s, which is 4.53 times the QPU access time. A quiet-host re-time of the same method scores 3 / 0 / 9 at 0.057 s.
- Verdict date: 2026-10-26.

## Limits and tiling
- Tileable: unknown. The zero-field sign-alignment protocol cannot run because the generator has 2,438 nonzero fields. The construction needs zero local fields so tile signs can align without changing energy. The measurement needed is a permitted zero-field tiled instance and a repeat of the paired experiment.
- Chaining adds nothing. Classical beats the call on every link: CPU anneals take 0.21 to 1.25 s on 6 threads, and the GPU wins at 131,072 sweeps or fewer.
- Seed: the CPU verdict method matches 12 of 12 models in one run and 3 of 12 in the re-time, 15 of 24 model-runs in total. The CPU gap to the QPU is small (a median of 1 to 2 energy units at the matched rungs).
- Depth: at 20 minutes one model reaches a CPU result 7 energy units below the QPU.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories under `/home/carback1/quip-data/regimes`: `qpu-f17/work/cells/ice-charge`, `new-pow/ice-charge`.
