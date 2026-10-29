<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Random-sign K4 tiles

This family keeps the 1,785 K4 tiles of the base ice problem but gives each of the six bonds of a tile its own random sign. A tile then has no gauge, so the strong layer is a frustrated spin glass with no ice manifold. It was screened to test whether removing the ice structure makes the problem harder for classical solvers. It did not. A single-flip GPU anneal and a CPU cluster anneal both reach the QPU result.

## Construction
Couplings are in units of the full-strength coupler (J = 1), with the milli value in a code span. The working graph has 4,575 qubits and 41,474 couplers.

- Strong layer: 1,785 K4 tiles. Each of the six bonds takes a random sign at 1 (`±1000`).
- Weak layer: every other coupler is +0.125 or −0.125 (`±125`).
- Fields: h 0. Distinct couplers: {±0.125, ±1}.
- First nonce: 41,474 couplers per model. Of these, 10,710 are at ±1 (5,440 at `-1` and 5,270 at +1) and 30,764 are at ±0.125 (15,407 at −0.125 and 15,357 at +0.125).
- Topology use: 100 percent of qubits and 100 percent of couplers.

## Compact representations
- Problem (256 bits: the nonce plus the generator version): summed tile terms `A/2 (sum g s - q)^2`. The terms depend on tile overlap. They also depend on random signs and on charges q.
- Solution (4,575 bits raw, compact size not measured): overlap and random signs cut tiles to 2 states or cancel bonds. Charge-2 tiles keep 4 states (1-in-4).
- Explicit size of the largest of 12 models: 41,474 couplers × 2 bits = 82,948 bits (4 values).

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `ice-random-k4` | Random-sign K4 bonds at `±1000`, glass `±125` | [`corpora/ice-random-k4.jsonl`](../../corpora/ice-random-k4.jsonl) | Closed | 3 | Tier 2 |

Tier 3 and tier 4 mean that a classical run inside 120 s beats the postprocessed QPU result on at least 60 percent of 30 or more common models. Tier 4 means the best classical run is also within five times the QPU wall time. Tier 1 and tier 2 mean no classical run beats the QPU in that way, and tier 1 means the QPU is also fast. New-pow Tier 1 means classical is strictly lower on a majority of models within 0.1 s. New-pow Tier 2 means parity within 20 min.

## Generator and postprocess
Generator: [`generate2.py`](../twisted-ice/sources/qpu-f17/generate2.py) builds `ice-random-k4` from the saved twisted ice models, giving each tetrahedron bond an independent random sign at 1,000 milli. Postprocess: `ice-tiles` ([`icetiles.py`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/icetiles.py), [`icetiles.c`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/icetiles.c), [`method.py`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/method.py)), with `anneal:sa=1,b0=2,fe=4,nat48=1` at 0.25 s of one core per call. It gains 1 (1) energy unit over greedy descent.

## Results
- Class and profile: not recorded. Tier 3. New-pow Tier 2.
- Energies, median over models, in energy units: QPU −6,888, QPU after postprocess −6,896, CPU −6,924, CUDA −6,908.
- Energy gap: +17 (8), QPU 0 / tie 0 / CPU 30, 118 s, 30 models.
- Best classical: CUDA at 6.1 times the QPU access time (1.8 times the QPU wall time). Rank by time to the postprocessed target: QPU (0.046 s) < CUDA (1.81 s) < CPU (7.96 s). The CUDA arm is a cluster anneal at 8,192 sweeps over 100 models, and the page marks it not reliable. A single-flip GPU at 32,768 sweeps matches 85 of 100 models (2.30 card-seconds per model).
- New-pow campaign (2026-10-20): 8 / 0 / 4 over 12 models for the single-flip population anneal (`glasssa`) at 8,192 sweeps. Median wall 0.289 s, which is 6.28 times the QPU access time.
- Verdict date: 2026-10-26.

## Limits and tiling
- Tileable: yes. All three tiled-generator conditions pass across 12 nonces. The largest energy delta is 6.5625 units per tile, with wins, ties, and losses of 8 / 2 / 2. The tile solvers reached 39 of 48 targets, and all four reached their targets in 4 of 12 runs.
- Chaining adds nothing. Classical beats the call on every link: CPU anneals take 0.21 to 1.25 s on 6 threads, and the GPU wins at 131,072 sweeps or fewer.
- Seed: the CUDA cluster anneal at 4,096 sweeps matches 7 of 12 models. The verdict CPU wall is 0.289 s with 8 of 12 models matching.
- Depth: at 20 minutes one model reaches a CPU result 40 energy units below the QPU.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories under `/home/carback1/quip-data/regimes`: `qpu-f17/work/cells/ice-random-k4`, `new-pow/ice-random-k4`.
