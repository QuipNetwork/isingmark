<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Twisted ice with sparse signed fields (F24)

This family adds sparse local fields to the glass-broken spin ice. A field pins one spin and breaks the global flip symmetry. The study asked whether a few strong fields stop the cluster moves that solve plain ice. On ice alone, field-only selection is a polynomial problem. The weak glass removes that shortcut.

## Construction
Couplings are in units of the full-strength coupler (J = 1), with the milli value in a code span. The working graph has 4,575 qubits and 41,474 couplers.

- Base: F12 twisted ice with the F11 r8 couplings. Strong bonds are at 1 (`1000`), and the glass is ±0.125 (`±125`).
- Fields: 285 spins per model carry a signed field. The cell `ice-fields-h500` uses ±0.500 (`±500`). The cell `ice-fields-h1000` uses ±1 (`±1000`). Field locations and signs are the same in both cells for each nonce.
- Distinct couplers: {±0.125, ±1}.
- Topology use: 100 percent of qubits and 100 percent of couplers.

## Compact representations
- Problem (256 bits: the nonce plus the generator version): F11 r8 couplings plus 285 fields of ±0.500 or ±1.
- Solution (4,575 bits, raw at one bit per used qubit, compact size not measured): a field-only selector on ice is a min-cost circulation (polynomial). The weak glass removes that.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `ice-fields-h1000` | 285 fields at ±1 (`±1000`), glass `±125` | [`corpora/ice-fields-h1000.jsonl`](../../corpora/ice-fields-h1000.jsonl) | Closed | 3 | Tier 2 |
| `ice-fields-h500` | 285 fields at ±0.500 (`±500`), glass `±125` | [`corpora/ice-fields-h500.jsonl`](../../corpora/ice-fields-h500.jsonl) | Closed | 3 | Tier 2 |

Tier 3 and tier 4 mean that a classical run inside 120 s beats the postprocessed QPU result on at least 60 percent of 30 or more common models. Tier 4 means the best classical run is also within five times the QPU wall time. Tier 1 and tier 2 mean no classical run beats the QPU in that way, and tier 1 means the QPU is also fast. New-pow Tier 1 means classical is strictly lower on a majority of models within 0.1 s. New-pow Tier 2 means parity within 20 min.

## Generator and postprocess
- Generator: [`ice_fields.py`](sources/qpu-suite/work/ice_fields.py) builds the field instance. It derives a field nonce from a SHA-256 of the nonce and a field domain tag. The base couplings come from the twisted ice generator ([`ice_twisted.py`](../../tools/structured/isingmark_structured/families/ice_twisted.py) in structured form).
- Postprocess: `ice-tiles` ([`icetiles.py`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/icetiles.py), [`icetiles.c`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/icetiles.c), [`method.py`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/method.py)), at 0.25 s of one core per call.

## Results
| Cell | Class | QPU / QPU pp / CPU / CUDA energy (units) | Energy gap | Best classical | Closed by |
| --- | --- | --- | --- | --- | --- |
| `ice-fields-h1000` | 1, classical beats cheaply | −4,774 / −4,779 / −4,791 / −4,777 | +6 (4), QPU 2 / tie 0 / CPU 28, 111 s, 30 models | CPU 91× (27×) | Single-flip GPU at 131,072 sweeps (8 of 12). The auxiliary 500 µs setting was unresolved at the stop. |
| `ice-fields-h500` | 3, QPU leads (summary row; see the note below) | −4,664 / −4,664 / −4,678 / −4,665 | +6 (3), QPU 3 / tie 0 / CUDA 97, 1.77 s, 100 models | CUDA 6.0× (1.8×) | Production fails the GPU gate at 19 of 24. The longer-call classical verdict was unmeasured when the campaign stopped. |

`ice-fields-h1000`:
- Profile: classical cheap and CUDA-bound. CUDA beats CPU. The best anneal time is 500 µs.
- Production beats both CPU screens on 12 of 12 models at 8,192 sweeps. The deep GPU rung is the strongest measured screen. Production wins 4 of 12 at 131,072 sweeps against the single-flip GPU.
- Strongest attack: 8 / 0 / 4 for GPU single flip at 131,072 sweeps, 131.1 s.
- New-pow campaign (2026-10-20): 12 / 0 / 0 for a gauge-aware Otsuka cluster anneal at 8,192 sweeps. Median wall 2.4 s, which is 52.1 times the QPU access time.

`ice-fields-h500`:
- Profile: QPU leads and CUDA-bound. Neither CPU nor CUDA beats the QPU in the suite run. The best anneal time is 500 µs.
- Both CPU screens (local blocks and ice loops) lose 12 of 12 at 8,192 sweeps. The field-capable tempering attack did not run.
- Production wins 19 of 24 against the single-flip GPU at 131,072 sweeps (0 ties, 5 losses). The 500 µs call wins 22 of 24. The 256-read reference wins 24 of 24.
- Strongest attack: 5 / 0 / 19 for GPU single flip at 131,072 sweeps, 131.5 s.
- New-pow campaign (2026-10-20): 22 / 0 / 2 over 24 models for an Otsuka cluster anneal with the fields in the heat bath at 8,192 sweeps. Median wall 4.19 s, which is 91.1 times the QPU access time.

Per-cell details:
- `ice-fields-h1000`: strong fields make the best states keep 2 to 10 tile defects joined by strings that follow the fields, so the best states are not pure ice states. Postprocess `anneal:b0=2,nat48=1` at 0.25 s (gain 3 (2)). The 2,000 µs call is below the production target on 11 of 12 models (median 3 units) and the 200 µs call on 10 of 12 (median 2). CUDA cluster at 4,096 sweeps matches 12 of 12. The CPU 30 s 100-model rung (27.0 s) reaches the target on a majority of models. At 1,200 s three models reach a CPU result with a median 24 units below the QPU (deepest 25). One CPU job needs 2.43 s per link.
- `ice-fields-h500`: the strong layer is under one global gauge per model. The 285 fields sit on spins that belong to two tiles each. Postprocess `anneal:b0=2,nat48=1` at 0.25 s (gain 3 (2)). The 500 µs call is below the production target on 22 of 24 models (median 3 units), and the 256-read reference on 23 of 24 (median 6). A CUDA cluster anneal at 16,384 sweeps matches 97 of 100 models in 1.77 card-seconds, and a single-flip GPU needs 262,144 sweeps for 21 of 24. The page text shows the CUDA cluster anneal matching 97 of 100 models, so the summary class 3 (QPU leads), which rests on the single-flip GPU screen alone, does not hold against it. The CPU 120 s 100-model rung (110 s) reaches the target on 29 of 30 models. At 1,200 s three models reach a CPU result with a median 24 units below the QPU (deepest 26). One CPU job needs 2.81 s per link.

Tier 3 for both cells, and New-pow Tier 2 for both. Verdict date: 2026-10-26.

## Limits and tiling
- Tileable: unknown for both cells.
- Chaining: a hash chain is possible. Per link, classical matches in 4.2 s (h500) and 2.4 s (h1000). A chain that sets fields from the previous solution would turn each link into a field cell. Field cells are the cheapest classical ice cells.
- The `ice-fields-h500` cell is the one place in this family where the suite run shows a QPU lead, and the new-pow Otsuka anneal with fields in the heat bath later reached parity or better on 22 of 24 models. Treat the QPU lead as open.
- Tiling detail: the zero-field sign-alignment protocol cannot run because each model has 285 nonzero fields. The measurement needed is a permitted zero-field tiled instance and a repeat of the paired experiment.
- Field strength: the verdict CPU wall is 4.19 s for ±0.500 (22 of 24 models match) and 2.40 s for ±1 (12 of 12).

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories under `/home/carback1/quip-data/regimes`: `qpu-suite/work/cells/ice-fields-h1000`, `new-pow/ice-fields-h1000`, `qpu-suite/work/cells/ice-fields-h500`, `new-pow/ice-fields-h500`.
