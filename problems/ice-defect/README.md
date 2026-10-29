<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Twisted ice with reversed-bond defects (F20 to F23)

This family starts from twisted spin ice (F12) and reverses one strong bond in a chosen fraction of the tetrahedra. A reversed bond leaves two local ground states in that tetrahedron instead of six, so the defects add rigid motifs that single-flip moves cross slowly. Six defect fractions were screened, from half the tetrahedra down to one in 64. The idea was that rare, rigid defects would slow classical solvers. Dense defects did not. Sparse defects did, but signed-block tempering still closed the family.

## Construction
Couplings are in units of the full-strength coupler (J = 1), with the milli value in a code span. The working graph has 4,575 qubits and 41,474 couplers.

- Base: the F12 twisted ice topology and magnitudes. Strong bonds are at 1 (`1000`), and the glass is ±0.125 (`±125`).
- Defect: one strong bond is reversed in a fraction p of the tetrahedra. Each changed tetrahedron has two local ground states instead of six.
- Fields: h 0. Distinct couplers: {±0.125, ±1}.
- Topology use: 100 percent of qubits and 100 percent of couplers.

## Compact representations
- Problem (256 bits: the nonce plus the generator version): twisted ice with one reversed bond in a fraction p of tiles.
- Solution (4,575 bits raw, compact size not measured, except `ice-defect-p03125` at 2,192 bits compact, mean per qualifying state, at most 2,292): a mixed rule with 6-state tiles at −2,000 and 2-state tiles at −4,000 (milli units). The lower bound is closed form. Attainment is not certified.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `ice-defect-p25` | Reversed bond in 1/4 of tiles (F20) | [`corpora/ice-defect-p25.jsonl`](../../corpora/ice-defect-p25.jsonl) | Closed | 3 | Tier 2 |
| `ice-defect-p015625` | Reversed bond in 1/64 of tiles (F23) | [`corpora/ice-defect-p015625.jsonl`](../../corpora/ice-defect-p015625.jsonl) | Closed | 4 | Tier 2 |
| `ice-defect-p125` | Reversed bond in 1/8 of tiles (F21) | [`corpora/ice-defect-p125.jsonl`](../../corpora/ice-defect-p125.jsonl) | Closed | 3 | Tier 2 |
| `ice-defect-p0625` | Reversed bond in 1/16 of tiles (F21) | [`corpora/ice-defect-p0625.jsonl`](../../corpora/ice-defect-p0625.jsonl) | Closed | 3 | Tier 2 |
| `ice-defect-p50` | Reversed bond in 1/2 of tiles (F20) | [`corpora/ice-defect-p50.jsonl`](../../corpora/ice-defect-p50.jsonl) | Closed | 4 | Tier 2 |
| `ice-defect-p03125` | Reversed bond in 1/32 of tiles (F23) | [`corpora/ice-defect-p03125.jsonl`](../../corpora/ice-defect-p03125.jsonl) | Closed | 4 | Tier 2 |

Tier 3 and tier 4 mean that a classical run inside 120 s beats the postprocessed QPU result on at least 60 percent of 30 or more common models. Tier 4 means the best classical run is also within five times the QPU wall time. Tier 1 and tier 2 mean no classical run beats the QPU in that way, and tier 1 means the QPU is also fast. New-pow Tier 1 means classical is strictly lower on a majority of models within 0.1 s. New-pow Tier 2 means parity within 20 min.

## Generator and postprocess
- Base instance: the twisted ice generator, imported in two forms. The structured port is [`ice_twisted.py`](../../tools/structured/isingmark_structured/families/ice_twisted.py). The original suite file is [`ice_twisted.py`](../twisted-ice/sources/qpu-suite/work/ice_twisted.py).
- Defect step (reversing one strong bond in a fraction p of tetrahedra), research root, not imported: `qpu-suite/work/ice_defect.py` (F20 cells `ice-defect-p25` and `ice-defect-p50`), `qpu-suite/work/ice_sparse_defect.py` (F21 cells `ice-defect-p125` and `ice-defect-p0625`), and `qpu-suite/work/ice_rare_defect.py` (F23 cells `ice-defect-p03125` and `ice-defect-p015625`). No cell record names a generator file, so this mapping follows the family code of each cell.
- Attack code used to close the family (Otsuka cluster anneal with exact defect bonds, signed conditional blocks with replica exchange, block tempering): not imported. The shared ice-tile postprocess is at `../shared/sources/new-pow/postprocess/optimizers/ice-tiles`. The postprocess setting for these cells: not stated in the source.

## Results
| Cell | Class | Faster device | QPU / QPU pp / CPU / CUDA energy (units) | Energy gap | Best classical | Closed by | New-pow wins / ties / losses, sweeps, wall (ratio to access) | Tileable |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `ice-defect-p50` | 1 | CUDA | −5,778 / −5,775 / −5,792 / −5,784 | +12 (6), QPU 1 / tie 0 / CUDA 99, 2.23 s, 100 models | CUDA 5.2× (1.5×) | Single-flip GPU at 32,768 sweeps (9 of 12 at production) | 8 / 0 / 4, 16,384 sweeps, 8.08 s (176×) | yes |
| `ice-defect-p25` | 1 | CUDA | −5,219 / −5,222 / −5,235 / −5,218 | +2 (7), QPU 11 / tie 0 / CPU 19, 63.9 s, 30 models | CUDA 32× (9.4×) | Single-flip GPU at 131,072 sweeps | 14 / 0 / 10, 12,288 sweeps, 6.92 s (150×) | yes |
| `ice-defect-p125` | 1 | CUDA | −4,917 / −4,922 / −4,937 / −4,918 | +8 (5), QPU 1 / tie 0 / CPU 29, 138 s, 30 models | CPU 465× (136×) | Single-flip GPU at 131,072 sweeps | 8 / 0 / 4, 8,192 sweeps, 4.62 s (101×) | no |
| `ice-defect-p0625` | 1 | both CPU and CUDA beat the QPU | −4,762 / −4,763 / −4,778 / −4,766 | +6 (4), QPU 5 / tie 1 / CUDA 94, 2.03 s, 100 models | CUDA 6.9× (2.0×) | Block tempering energy gate at 131,072 sweeps | 24 / 0 / 0, 24,576 sweeps, 9.77 s (212×) | no |
| `ice-defect-p03125` | 2 | CPU | −4,686 / −4,687 / −4,698 / −4,689 | +8 (3), QPU 0 / tie 0 / CUDA 100, 3.11 s, 100 models | CUDA 3.9× (2.0×) | Signed-block energy gate at 131,072 sweeps | 24 / 0 / 0, 32,768 sweeps, 12.6 s (273×) | yes |
| `ice-defect-p015625` | 2 | CPU | −4,644 / −4,644 / −4,657 / −4,648 | +7 (3), QPU 4 / tie 0 / CUDA 96, 2.96 s, 100 models | CUDA 3.7× (1.9×) | Signed-block energy gate at 131,072 sweeps | 24 / 0 / 0, 32,768 sweeps, 12.7 s (276×) | yes |

Class 1 means classical beats the production call cheaply, and class 2 means classical beats it at cost. The best anneal time is 2,000 µs for `ice-defect-p25`, `ice-defect-p03125`, `ice-defect-p015625`, and `ice-defect-p50`. It is 500 µs for `ice-defect-p125` and `ice-defect-p0625`. Verdict date for all six cells: 2026-10-26.

Profile. The cells `ice-defect-p03125` and `ice-defect-p015625` have the profile QPU leads and CPU competitive (class 2, sequential CPU method best). The cell `ice-defect-p0625` is classical cheap and CPU competitive. The other three cells are classical cheap and CUDA-bound.

QPU against single-flip GPU, production call at 131,072 sweeps:
- `ice-defect-p25`: wins 2 of 24. The selected 2,000 µs call wins 11 of 24, and the reference wins 17 of 24. The GPU beats production on 22 of 24, and the CPU tetrahedron moves lose all 12 screened models.
- `ice-defect-p125`: wins 0 of 12. The GPU rejects every setting on the first 12 models (60.8 s per model).
- `ice-defect-p0625`: wins 5 of 24. The selected 500 µs call wins 21 of 24, and the deep reference wins 23 of 24. Strong-graph block tempering at 131,072 sweeps matches 24 production, 24 selected, and 20 reference models among 24 (24 / 0 / 0 in 851.1 s).
- `ice-defect-p03125`: wins 15 of 24. Signed conditional blocks with replica exchange at 131,072 sweeps beat production on 24 of 24 (1,407.5 s per model). The selected call keeps 5 wins among 24. The GPU at 131,072 sweeps scores 9 / 0 / 15.
- `ice-defect-p015625`: wins 13 of 24. Signed blocks with replica exchange at 131,072 sweeps beat production on 22 of 24 (1,012.4 s per model). The selected call keeps 7 wins among 24. The GPU scores 11 / 0 / 13.
- `ice-defect-p50`: wins 0 of 12. The deepest GPU rung supplies the rejection, and CPU tetrahedron moves lose all 12 models. The GPU scores 12 / 0 / 0 at 131,072 sweeps (58.3 s) and 9 / 0 / 3 at 32,768 sweeps (14.5 s).

Per-cell details:
- `ice-defect-p50`: the nonce reverses one edge in a tile with probability 1/2, which gave 892 of 1,785 tiles for the first nonce. Postprocess `anneal:b0=2,cross=0.2,nat48=1` at 0.25 s (gain 3 (2)). The 2,000 µs call is below the production target on 12 of 12 models (median 8 units). A single-flip GPU anneal at 32,768 sweeps matches 9 of 12 models and 50 of 100 on the 100-model batch (not reliable). The CUDA cluster anneal at 16,384 sweeps matches 99 of 100. Tiling: all three conditions pass. The largest delta 11.875 units per tile. Wins 9 / 1 / 2. Tile solvers reached 24 of 48 targets. One CPU job needs 10.7 s per link. At 1,200 s one model reaches a CPU result 29 units below the QPU.
- `ice-defect-p25`: about 446 of 1,785 tiles carry a reversed bond. Postprocess `anneal:b0=2,cross=0.2,nat48=1` at 0.25 s (gain 3 (3)). The 48-read 2,000 µs call is below the production target on 21 of 24 models (median 6 units), and the 256-read reference on 23 of 24 (median 9). CPU matches over two verdict-kernel runs: 31 of 48 model-runs. CUDA cluster at 4,096 sweeps matches 19 of 24. Tiling: the largest delta 9.9375 units per tile. Wins 11 / 0 / 1. Tile solvers reached 21 of 48 targets. One CPU job needs 9.12 s per link. At 1,200 s one model reaches a CPU result 20 units below the QPU.
- `ice-defect-p125`: 223 of 1,785 tiles carry a reversed bond. Postprocess `anneal:b0=2,cross=0.2,nat48=1` at 0.25 s (gain 4 (3)). The 500 µs call is below the production target on 11 of 12 models (median 5 units), and the 2,000 µs call on 9 of 12 (median 3). CPU matches over two verdict-kernel runs: 17 of 24 model-runs. CUDA cluster at 4,096 sweeps matches 12 of 12. The CUDA single-flip ladder has no majority rung on 30 models, and its deepest rung needs more than 15.0 card-seconds. Tiling: condition 2 fails. The largest delta 11.6875 units per tile. Tiled wins 12 / 0 / 0. Tile solvers reached 33 of 48 targets. One CPU job needs 6.17 s per link. At 1,200 s one model reaches a CPU result 25 units below the QPU.
- `ice-defect-p0625`: 111 of 1,785 tiles carry a reversed bond, and the model has no planted pairs. Postprocess `anneal:b0=2,cross=0.2,nat48=1` at 0.25 s (gain 3 (2)). The 500 µs call is below the production target on 23 of 24 models (median 6 units), the 2,000 µs call on 12 of 12 (median 4), and the 256-read reference on 24 of 24 (median 10). CUDA cluster at 4,096 sweeps matches 20 of 24. Tiling: condition 2 fails. The largest delta 6.25 units per tile. Tiled wins 12 / 0 / 0. Tile solvers reached 35 of 48 targets. One CPU job needs 6.45 s per link. At 1,200 s three models reach a CPU result with a median 30 units below the QPU (deepest 41).
- `ice-defect-p03125`: 55 of 1,785 tiles carry a reversed bond. Postprocess `anneal:b0=2,cross=0.2,nat48=1` at 0.76 s (gain 4 (2)). The first-call best of the first nonce codes to 2,216 bits (a header of 86 bits listing 9 tile defects, 471 free spins), and the mean over 147 qualifying states is 2,192 bits. The 48-read 2,000 µs call is below the production target on 23 of 24 models (median 5 units), and the 256-read reference on 24 of 24 (median 9). CUDA cluster at 4,096 sweeps matches 16 of 24, and the single-flip GPU needs 262,144 sweeps for 20 of 24. Tiling: the largest delta 11.625 units per tile. Wins 10 / 0 / 2. Tile solvers reached 35 of 48 targets. One CPU job needs 7.96 s per link. At 1,200 s three models reach a CPU result with a median 21 units below the QPU (deepest 26).
- `ice-defect-p015625`: 27 of 1,785 tiles carry a reversed bond. Postprocess `anneal:b0=2,cross=0.2,nat48=1` at 0.76 s (gain 4 (2)). The 48-read 2,000 µs call is below the production target on 19 of 24 models (median 5 units), and the 256-read reference on 24 of 24 (median 9). CPU matches over two verdict-kernel runs: 47 of 48 model-runs. CUDA cluster at 8,192 sweeps matches 23 of 24. Tiling: the largest delta 8.6875 units per tile. Wins 11 / 0 / 1. Tile solvers reached 38 of 48 targets. One CPU job needs 7.13 s per link. At 1,200 s three models reach a CPU result with a median 22 units below the QPU (deepest 25).

Pattern. Dense defects (1/2, 1/4, 1/8) fall to a single-flip GPU anneal at 131,072 sweeps or fewer. Sparse defects (1/16, 1/32, 1/64) need signed-block tempering at 131,072 sweeps, which costs 851.1 to 1,407.5 s per model. In the new-pow campaign, the Otsuka cluster anneal with exact defect bonds wins all 24 models for the three sparsest cells.

## Limits and tiling
- Tileable: yes for `ice-defect-p25`, `ice-defect-p50`, `ice-defect-p03125`, and `ice-defect-p015625`. No for `ice-defect-p125` and `ice-defect-p0625`.
- Chaining: a hash chain is possible, but per link classical matches in 4.6 to 12.7 s on 6 threads. Defects add rigid motifs that a cut between chips would split. That hurts a QPU block solve more than a classical one.
- The defect fraction does not restore a QPU lead. The two sparsest cells keep the QPU ahead of a single-flip GPU anneal on 13 and 15 of 24 models, but a tailored sequential method closes them.
- Defect density: the verdict CPU wall by share of tiles with a reversed bond is 12.7 s for 1/64 (24 of 24 models match), 12.6 s for 1/32 (24 of 24), 9.77 s for 1/16 (24 of 24), 4.62 s for 1/8 (8 of 12), 6.92 s for 1/4 (14 of 24), and 8.08 s for 1/2 (8 of 12).
- At p = 1/2 rigid components conflict, so the strong floor is not reached.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories under `/home/carback1/quip-data/regimes`: `qpu-suite/work/cells/ice-defect-p25`, `new-pow/ice-defect-p25`, `qpu-suite/work/cells/ice-defect-p015625`, `new-pow/ice-defect-p015625`, `qpu-suite/work/cells/ice-defect-p125`, `new-pow/ice-defect-p125`, `qpu-suite/work/cells/ice-defect-p0625`, `new-pow/ice-defect-p0625`, `qpu-suite/work/cells/ice-defect-p50`, `new-pow/ice-defect-p50`, `qpu-suite/work/cells/ice-defect-p03125`, `new-pow/ice-defect-p03125`.
