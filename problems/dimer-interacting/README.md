<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Interacting dimers

Interacting dimers place one spin on each edge of a template graph built from the chip K4 tiles. A strong vertex term asks for exactly one dimer at every vertex, so the ground states are perfect matchings of the tetrahedron network. A weaker pair term between dimer spins with no common vertex selects among the matchings. Without the pair term the problem is a weighted matching that a polynomial-time algorithm solves. The pair term blocks that reduction. The family was screened as a dimer model that shares tiles with spin ice.

## Construction
The strong layer is the vertex term 0.250 (250 milli, in units of the full-strength coupler). Each template vertex is one native K4, and each spin is a dimer position on the edge between its vertices. Four spins at a degree-4 vertex couple pairwise at `+0.125`. The vertex is satisfied if and only if exactly one of its dimers is present. The weak layer is the pair term 0.248 with sign σ = `±1`, placed on a topology coupler between two dimer spins with no common vertex. It gives a coupler of `±0.062`. Each spin carries at most one pair term.

For `dimers-interacting` the first model has 1,748 vertices, 2,990 dimer spins and 1,381 pair terms, about 0.46 per spin. The two layers have close weights, so a missing dimer costs about one pair term and the best states hold 40 to 55 vertex defects. Fields are h `-0.562` to `+0.062` (14 values) and couplers are J {`±0.062`, `+0.125`}.

For `dimers-fullchip` the template spans the full chip. The strong layer is a vertex penalty on the tetrahedron network. Each of the 2,470 vertices of the first nonce penalizes any number of chosen dimer edges other than one, and the penalty becomes 12,190 couplers of 0.124 (`124` milli) between the dimer variables that meet at a vertex. The weak layer is 2,096 pair terms of 0.062 (1,020 with sign `+1` and 1,076 with sign `-1`). The 4,472 dimer variables stand on 4,575 qubits. The nonce seeds the pair terms and their signs. Fields are h `-0.558` to `+0.062` (10 values) and couplers are J {`±0.062`, `+0.124`}.

Topology use (qubits / couplers / topology): `dimers-interacting` 65.7% / 21.9% / 26.3%. `dimers-fullchip` 98.3% / 34.7% / 41%.

## Compact representations
- Problem: 256 bits (nonce, plus the generator version). Energy is 0.250(sum x − 1)^2 per vertex plus 0.248 σ x_e x_f.
- Solution: raw bits are 3,004 (`dimers-interacting`) and 4,495 (`dimers-fullchip`), one per used qubit. The compact form is not measured. The answer set is the perfect matchings of the tetrahedron network, a permanent count with no Pfaffian because the network is not planar. The pair terms select among them.
- Compact formula: yes for `dimers-interacting`, as 250(sum x − 1)^2 per template vertex plus 248 σ x_e x_f pair terms.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `dimers-fullchip` | interacting dimers on the full chip | [`corpora/dimers-fullchip.jsonl`](../../corpora/dimers-fullchip.jsonl) | Closed | 4 | Tier 1 |
| `dimers-interacting` | 0.250 vertices, 0.248 pairs | [`corpora/dimers-interacting.jsonl`](../../corpora/dimers-interacting.jsonl) | Closed | 3 | Tier 2 |

## Generator and postprocess
The generators are not imported. The research generators are `families/dimers-interacting/generate.py` and `families/dimers-fullchip/generate.py` under the new-pow research tree. The cell data derives the K4 matching, the vertex set and every pair sign from the nonce through purpose digests. No source file in `sources/` belongs to this family. The production postprocess is `dimers` plus `worm` (0.25 s of one core per call for `dimers-interacting`, 0.76 s for `dimers-fullchip`). The postprocess code is not imported.

## Results
| Measure | `dimers-fullchip` | `dimers-interacting` |
| --- | --- | --- |
| Rank | QPU (0.048 s) < CUDA (>1.71 s) < CPU (1.98 s) | QPU (0.071 s) < CUDA (>0.658 s) < CPU (1.80 s) |
| Best classical (CPU gap) | CPU 2.5× (1.3×) | CPU 6.1× (2.0×) |
| CUDA gap | >2.1× (>1.1×) | >2.2× (>0.7×) |
| Energy gap | `+1` (0), QPU 0 / tie 0 / CPU 30, 120 s, 30 models | `+1` (0), QPU 0 / tie 0 / CPU 30, 29.8 s, 30 models |
| GSE medians (QPU / CUDA / CPU) | `-1,100` / `-1,100` / `-1,102` | `-699` / `-699` / `-701` |
| Deep sweep | CPU 2 (1) / depth-10 / 10 s | >CPU 1 (0) / `depth-1200s-6t` / 1,090 s |
| New-pow wins | 8 / 1 / 3 of 12, cold-start dimer anneal shared by 6 threads | 10 / 0 / 2 of 12, compiled single-flip anneal over dimer variables, 12,000 sweeps |
| New-pow wall, ratio vs access | 0.097 s, 2.1 | 0.224 s, 4.9 |
| Runs (QPU / CUDA / CPU) | 132 / 66 / 340 | 132 / 54 / 268 |

For `dimers-fullchip` the QPU TTS is 0.048 s: 0.046 s per call divided by the measured p = 0.95 (median over 12 models, pooled 0.77 from 92 of 120 calls). The repeat-call spread of the best is 1 energy unit (1 to 1 over 12 models). The CPU verdict kernel matches the QPU target on 9 of 12 models. The first single-flip GPU rung with a majority is 32,768 sweeps (10 of 12 models, 1.69 card-s). The deepest classical result at 20 minutes is a CPU median of `-2` energy units on 3 models.

For `dimers-interacting` the QPU TTS is 0.071 s: 0.046 s per call divided by the measured per-call success probability p = 0.65 (median over 12 models, pooled 0.58 from 70 of 120 calls). The repeat-call spread of the best is 1 energy unit (0 to 1 over 12 models). The CPU verdict kernel matches the QPU target on 10 of 12 models in both runs (20 of 24 model-runs). The first single-flip GPU rung with a majority is 16,384 sweeps, 9 of 12 models. The deepest classical result at 20 minutes is a CPU median of `-2` energy units on 3 models.

## Limits and tiling
Tileable: unknown for both cells. The tiled-generator protocol is blocked because the construction has nonzero local fields (2,987 for `dimers-interacting`, 4,471 for `dimers-fullchip`) and tiling needs zero fields. A matching is the 1-in-4 charge rule on the same tiles as ice, so it can share spins with an ice layer.

| Item | `dimers-fullchip` | `dimers-interacting` |
| --- | --- | --- |
| Chaining analysis | none | uncaptured, per-link ratio unknown |
| Fresh call reaches the first-call target | 95 percent (median per model) | 65 percent (median per model) |
| QPU cost per link at 0.1 s per call | 0.105 s | 0.154 s |
| One CPU job per link | not recorded | 0.619 s |
| Fleet per link at 100 or 1,000 jobs | not recorded | 0.516 s |

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories: `regimes/new-pow/dimers-fullchip`, `regimes/new-pow/dimers-interacting`, `regimes/new-pow/retime/tier2/dimers-interacting`.
