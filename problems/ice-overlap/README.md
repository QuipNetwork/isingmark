<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Edge-overlapping ice tiles (F22)

This family lets the strong K4 tiles share couplers. Where two tiles share a coupler, their gauges either add to a doubled bond or cancel. The doubled and cancelled couplers break the sixfold ice manifold, so the strong layer acts as a dense ±J glass instead of as ice. The cells vary how many tiles may overlap, from at most two tiles per edge to every native K4 on the working graph. They were screened to see whether a denser, more frustrated tile layer stays hard. It did not. A single-flip GPU anneal closed every cell.

## Construction
Couplings are in units of the full-strength coupler (J = 1), with the milli value in a code span. The working graph has 4,575 qubits and 41,474 couplers.

- `ice-overlap-c2`: native tetrahedra share couplers, with at most 2 on each edge. Tetrahedron bonds are at 0.500 (`+500`). The background is ±0.062 (`±62`). The source text says "per spin" in one place and per edge in another. This README uses per edge. Distinct couplers: {±0.062, ±0.500, ±1}.
- `ice-overlap-c3`: at most 3 tetrahedra per edge. Tetrahedron bonds are at 0.333 (`+333`), and the background is ±0.042 (`±42`). Distinct couplers: {±0.042, ±0.333, ±0.666, ±0.999}.
- `ice-all-k4`: every K4 of the working graph, 6,822 tiles at 0.250 each, with its own random gauge. The bond of tile t is `0.250 g_ta g_tb`, and the bonds add where tiles share a coupler, up to 8 tiles on one spin. Opposite contributions cancel, and the first nonce loses 1,075 couplers this way. Every coupler outside the tiles is ±0.031. The first model has 40,399 nonzero couplers: 295 at ±1, 243 at ±0.750, 1,365 at ±0.500, 28,149 at ±0.250, and 10,347 at ±0.031. Ten distinct values occur.
- `ice-overlap2`: about 2,550 K4 tiles at 0.500 per edge, each with its own gauge, and one spin may sit in up to three tiles. The remaining couplers are ±0.031. Between 700 and 775 couplers per model cancel. The first nonce gives 40,713 nonzero couplers: 757 at ±1, 12,324 at ±0.500, and 27,632 at ±0.031.
- Fields: h 0 for every cell. Topology use: 100 percent of qubits. Coupler use is 100 percent for c2 and c3, 97.5 percent for `ice-all-k4`, and 98.3 percent for `ice-overlap2`. Topology use is 97.8 percent for `ice-all-k4` and 98.5 percent for `ice-overlap2`.

## Compact representations
- Problem (256 bits: the nonce plus the generator version): summed tile terms `A/2 (sum g s - q)^2`. The terms depend on tile overlap. They also depend on random signs and on charges q.
- Solution (4,575 bits raw, compact size not measured): overlap and random signs cut tiles to 2 states or cancel bonds. Charge-2 tiles keep 4 states (1-in-4).
- Explicit sizes of the largest of 12 models: `ice-all-k4` is 40,445 couplers × 4 bits = 161,780 bits (10 values), and `ice-overlap2` is 40,772 couplers × 3 bits = 122,316 bits (6 values).

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `ice-overlap-c2` | At most 2 tiles per edge, tile bonds `+500`, background `±62` | [`corpora/ice-overlap-c2.jsonl`](../../corpora/ice-overlap-c2.jsonl) | Closed | 3 | Tier 2 |
| `ice-overlap-c3` | At most 3 tiles per edge, tile bonds `+333`, background `±42` | [`corpora/ice-overlap-c3.jsonl`](../../corpora/ice-overlap-c3.jsonl) | Closed | 3 | Tier 2 |
| `ice-all-k4` | Every native K4 at `250`, glass `±31` | [`corpora/ice-all-k4.jsonl`](../../corpora/ice-all-k4.jsonl) | Closed | 3 | Tier 2 |
| `ice-overlap2` | Overlapping tiles at `500`, glass `±31` | [`corpora/ice-overlap2.jsonl`](../../corpora/ice-overlap2.jsonl) | Closed | 3 | Tier 2 |

Tier 3 and tier 4 mean that a classical run inside 120 s beats the postprocessed QPU result on at least 60 percent of 30 or more common models. Tier 4 means the best classical run is also within five times the QPU wall time. Tier 1 and tier 2 mean no classical run beats the QPU in that way, and tier 1 means the QPU is also fast. New-pow Tier 1 means classical is strictly lower on a majority of models within 0.1 s. New-pow Tier 2 means parity within 20 min.

## Generator and postprocess
- `ice-all-k4`: built by `qpu-f17/generate3.py`, imported as [`generate3.py`](../charged-ice/sources/qpu-f17/generate3.py) under the charged-ice family. The nonce seeds the gauges and the glass signs.
- `ice-overlap2`: the same [`generate3.py`](../charged-ice/sources/qpu-f17/generate3.py). It accepts K4s in nonce order while every coupler is used by at most two tiles.
- `ice-overlap-c2` and `ice-overlap-c3`: `qpu-suite/work/ice_overlap.py` (research root), which is not imported.
- No cell record names a generator file, so these mappings follow the family code of each cell.
- Postprocess: `ice-tiles` ([`icetiles.py`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/icetiles.py), [`icetiles.c`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/icetiles.c), [`method.py`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/method.py)). For `ice-all-k4` the setting is `anneal:sa=1,b0=2,len=64,fe=16,nat48=1`. For `ice-overlap2` it is `anneal:sa=1,b0=2,len=32,fe=4,nat48=1`. Both use 0.25 s of one core per call. The setting for c2 and c3: not stated in the source.

## Results
| Cell | QPU / QPU pp / CPU / CUDA energy (units) | Energy gap | Best classical | New-pow wins / ties / losses, method, wall (ratio to access) | Tileable |
| --- | --- | --- | --- | --- | --- |
| `ice-overlap-c2` | −3,465 / −3,464 / −3,480 / −3,468 | +9 (4), QPU 0 / tie 0 / CPU 30, 119 s, 30 models | CUDA 36× (10×) | 12 / 0 / 0, single-flip population anneal (`glasssa`), 32,768 sweeps, 1.21 s (26.3×) | no |
| `ice-overlap-c3` | −2,946 / −2,948 / −2,960 / −2,953 | +8 (2), QPU 0 / tie 0 / CPU 30, 143 s, 30 models | CUDA 17× (4.9×) | 12 / 0 / 0, `glasssa`, 16,384 sweeps, 0.643 s (14×) | no |
| `ice-all-k4` | −2,625 / −2,641 / −2,653 / −2,644 | +5 (3), QPU 0 / tie 0 / CPU 30, 65.1 s, 30 models | CUDA 8.0× (2.4×) | 12 / 0 / 0, `glasssa`, 8,192 sweeps, 0.256 s (5.56×) | no |
| `ice-overlap2` | −2,640 / −2,659 / −2,675 / −2,662 | +6 (3), QPU 0 / tie 0 / CUDA 30, 37.8 s, 30 models | CUDA 128× (37×) | 12 / 0 / 0, `glasssa`, 32,768 sweeps, 1.25 s (27.3×) | yes |

- `ice-overlap-c2` and `ice-overlap-c3`: both are class 1 (classical beats cheaply). The profile is classical cheap and CUDA-bound, and CUDA beats CPU. The best anneal time is 2,000 µs. A single-spin GPU anneal rejects every setting of c2. The CPU block screen loses most comparisons. Production wins 0 of 12 against the single-flip GPU at 131,072 sweeps for both cells. The selected 2,000 µs call wins 5 of 12 for c2 and 0 of 12 for c3. The GPU at 32,768 sweeps wins 12 of 12 for c3 (14.4 s per model). Closed by: single-flip GPU at 32,768 sweeps (production) and 131,072 sweeps (longer calls) for c2, and at 32,768 sweeps for c3.
- `ice-all-k4` and `ice-overlap2`: class and profile are not recorded. The rank by time to the postprocessed target for `ice-all-k4` is QPU (0.046 s) < CUDA (2.37 s) < CPU (7.80 s). The rank for `ice-overlap2` is QPU (0.046 s) < CUDA (37.8 s) < CPU (151 s).
Per-cell details for c2 and c3:
- `ice-overlap-c2`: 3,480 four-spin tiles, each with its own gauge and 0.500 on each of its six edges, with at most two tiles per edge. A shared edge sums to ±1 when the two tile terms agree and cancels when they disagree. The first nonce has 3,214 shared edges, of which 1,572 sum to ±1 and 1,642 cancel to the weak layer. The first model has 1,572 couplers at ±1, 14,452 at ±0.500, and 25,450 at ±0.062. Postprocess `anneal:sa=1,b0=2,len=64,fe=4,nat48=1` at 0.25 s (gain 1 (1)). All three deep captures (200, 500, and 2,000 µs) are below the production target on 12 of 12 models, with a median of 5, 8, and 11 units. A CUDA cluster anneal at 16,384 sweeps matches 11 of 12 models, and a single-flip GPU needs 262,144 sweeps for 97 of 100. Tiling: condition 2 fails. The largest delta 5.898 units per tile. Tiled wins 12 / 0 / 0. Tile solvers reached 45 of 48 targets. One CPU job needs 1.22 s per link. At 1,200 s one model reaches a CPU result 32 units below the QPU.
- `ice-overlap-c3`: 5,142 K4 tiles at 0.333, each with its own random gauge, so its six bonds are `+0.333 g_a g_b`. The generator caps the load at 3 tiles per physical edge, and the summed bonds on a shared edge reach 0.333, 0.666, or 0.999. 367 tile bonds cancel to zero. The first model has 705 couplers at ±0.999, 375 at ±0.666, 22,935 at ±0.333, and 17,459 at ±0.042. The explicit form is 41,474 couplers × 3 bits = 124,422 bits (8 values occur). Postprocess `anneal:sa=1,b0=2,len=64,fe=4,nat48=1` at 0.25 s (gain 1 (0)). The 2,000 µs call is below the production target on 12 of 12 models (median 10 units), the 500 µs call on 12 of 12 (median 5), and the 200 µs call on 9 of 12 (median 2). A CUDA cluster anneal at 32,768 sweeps matches 94 of 100 models, and a single-flip GPU at 32,768 sweeps matches 12 of 12. Tiling: condition 2 fails. The largest delta 4.284 units per tile. Tiled wins 12 / 0 / 0. Tile solvers reached 48 of 48 targets. One CPU job needs 0.637 s per link. At 1,200 s one model reaches a CPU result 31 units below the QPU.

- Tiers: tier 3 for all four cells. Every cell has New-pow Tier 2. Verdict date: 2026-10-26.

## Limits and tiling
- Overlap does not help the QPU. The verdict CPU wall by tile layout is 1.21 s for c2 (12 of 12 models match), 0.643 s for c3 (12 of 12), 1.25 s for `ice-overlap2` (12 of 12), and 0.256 s for `ice-all-k4` (12 of 12).
- Chaining adds nothing. Classical beats the call on every link: CPU anneals take 0.21 to 1.25 s on 6 threads, and the GPU wins at 131,072 sweeps or fewer.
- Tileable: yes for `ice-overlap2` and no for `ice-overlap-c2`, `ice-overlap-c3`, and `ice-all-k4`. For `ice-all-k4`, condition 2 fails: the largest energy delta is 4.0895 units per tile, with tiled wins, ties, and direct wins of 12 / 0 / 0. For `ice-overlap2` all three conditions pass, with a largest delta of 6.2125 units per tile and wins of 11 / 0 / 1.
- Depth: at 20 minutes one `ice-all-k4` model reaches a CPU result 29 energy units below the QPU, and one `ice-overlap2` model reaches 42 units below.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories under `/home/carback1/quip-data/regimes`: `qpu-suite/work/cells/ice-overlap-c2`, `new-pow/ice-overlap-c2`, `qpu-suite/work/cells/ice-overlap-c3`, `new-pow/ice-overlap-c3`, `qpu-f17/work/cells/ice-all-k4`, `new-pow/ice-all-k4`, `qpu-f17/work/cells/ice-overlap2`, `new-pow/ice-overlap2`.
