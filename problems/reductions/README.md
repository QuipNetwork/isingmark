<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Combinatorial problem reductions on the chip

This family groups six encodings of classic combinatorial problems as Ising models on the chip graph. Each encoding uses strong gadget couplings to enforce constraints and weak fields or couplings to set costs or choose among solutions. The six are native OR-SAT clauses, Tait edge coloring, two-commodity routing, exact cover, signed not-all-equal triangles, and native weighted independent set. The family was screened to ask whether the chip handles problems that come from real reductions. Exact solvers (MILP, min-cost circulation, bucket elimination) and tuned annealers close all six.

## Construction
All values are in units of the full-strength coupler, with milli units in code spans where useful.

### Native OR-SAT clauses (`or-sat-native`)
The strong layer is 1,500 OR clauses in the first model. Each clause sits on a native K4 with three data spins u, v and w and one private auxiliary spin a. After taking the minimum over a, the gadget costs 0.496 (`496`) when all three literals are false and 0 otherwise. The weak layer is a field of `+0.031` or `-0.031` on every data spin. The clauses sum these into fields of `±0.031` to `±0.899` on 3,612 qubits. The nonce fixes the structure through the order of candidate K4 tiles and the draws for literal signs and field signs. The generator grows one connected instance by reusing data spins. The first model has 5,897 nonzero couplers: 2 at `±0.744`, 3,027 at `±0.496`, 1,368 at `±0.248` and 1,500 at `±0.124`. Random 3-SAT at this clause density has many satisfying states, so the clauses barely restrict the fields. An exact MILP proves the best value in 4 to 30 s, and every best state meets every clause. Fields h `-0.899` to `+0.899` (31 values), couplers J {`±0.124`, `±0.248`, `±0.496`, `+0.744`}. Topology use: 79.9% of qubits, 14.5% of couplers, 21% of the topology.

### Tait edge coloring (`tait-coloring`)
The strong layer is a pair of parity gadgets at each of 424 honeycomb vertices, one for bit layer a and one for bit layer b. Each gadget has a penalty of 0.248 per odd vertex. Each of the 501 full edges adds a 0.496 penalty on the color (0, 0), and its b bit is a two-spin chain at `-0.652`. A state with zero penalty is a Tait 3-edge coloring. The weak layer is a field of `+0.031` or `-0.031` on the 2,043 data bits. The honeycomb template is the same for every nonce, and the nonce draws only the field signs. A model has 6,090 nonzero couplers: 501 at `-0.652`, 2,544 at `-0.248` and 3,045 at `+0.124`. The generator plants no coloring. Fields h {`±0.031`, `-0.093`, `-0.124`, `-0.155`, `+0.248`}, couplers J {`+0.124`, `-0.248`, `-0.652`}. Topology use: 63.2% / 14.7% / 19.5%.

### Two-commodity routing (`two-commodity-routing`)
The strong layer is one ferromagnetic chain per arc. Each chain joins two spins at about 0.931 (880 couplers in all), and its strength exceeds the largest non-chain local field on any chain spin. The weak layer holds the other 5,468 couplers at `±0.124`. They carry the node conservation terms and a capacity term of 0.496 on each arc where the two commodities cross. Arc costs of 0.062 or 0.124 become fields on 2,059 qubits. The nonce draws arc costs and the 15 source and sink pairs of each commodity through SHAKE256. The template of 505 nodes and 880 arcs is the same for every nonce. The generator draws again with the next trial tag when the terminals admit no flow. Fields h `-0.682` to `+0.372` (24 values), couplers J {`±0.124`, `-0.931`}. Topology use: 57.7% / 15.3% / 19.5%.

### Exact cover (`exact-cover-mixed`)
The strong layer is a set of cardinality factors on native K4s. Each factor requires exactly k of its d spins to be 1 and costs 0.250 (sum x − k)^2. The types (d, k) = (3, 1), (4, 1) and (4, 2) are drawn with equal chance for each of the 6,822 candidate K4s. A three-spin factor is a triangle inside its K4. Every factor coupler is 0.125, and a coupler that two factors share is 0.250. The weak layer is an independent field of `+0.062` or `-0.062` on every constrained spin. The generator accepts candidates in priority-digest order from SHAKE256 draws over the nonce, with a spin in at most three factors and a coupler in at most two. The first nonce gives 2,770 factors (1,029 of type (3, 1), 871 of type (4, 1) and 870 of type (4, 2)) on 12,343 couplers, of which 1,190 carry two factors. Fields h `-0.812` to `+0.062` (14 values), couplers J {`+0.125`, `+0.250`}. Topology use: 89.4% / 30.1% / 35.9%.

### Signed not-all-equal triangles (`nae3-signed`)
The strong layer is 5,495 native triangles of the Zephyr graph in the first model. Each triangle carries three literal signs g and puts 0.500 times the product of two signs on each of its three couplers. Six of the eight spin states sit at the floor, and the two states with all signed spins equal cost 2 units more. The weak layer gives every other coupler between two triangle spins a random sign at 0.062. The nonce draws the triangle priority order, the literal signs and the weak signs through SHAKE256. Each model has 39,852 nonzero couplers: 1,322 at `±1` where two triangles share a coupler with equal sign products, 11,061 at `±0.500` and 27,469 at `±0.062`. Couplers where two triangles cancel leave the model. Fields are 0 and couplers are J {`±0.062`, `±0.500`, `±1`}. Topology use: 99.9% / 96.6% / 96.9%.

### Native weighted independent set (`wmis-native`)
The energy is a weighted maximum independent set on the full working graph, E = −sum_i w_i x_i + P sum_ij x_i x_j with x = (1 − s)/2. The model uses every qubit and every coupler of the live topology, with no auxiliaries. The nonce draws an integer v_i from 1 to 99 per qubit through framed SHAKE256 draws, and the weight is w_i = 2 v_i. The penalty P = 200 exceeds the largest weight of 198, so every best state is an independent set. In Ising terms every coupler is `+0.050` and each field is 0.001 (v_i − 50 deg_i), between `-0.999` and `-0.302`. The weight signal is 0.001 to 0.099 on top of a field of up to `-1`, so analog field error on the QPU is the same size as the weight differences. Fields h `-0.999` to `-0.331` (505 values), couplers J `+0.050`. Topology use: 100% / 100% / 100%.

## Compact representations
- Problem, all cells but `wmis-native`: 256 bits (the nonce and the generator version). The energy sums squared cardinality, parity or conservation terms per factor. For `nae3-signed` each triangle is 0.250[(sum g s)^2 − 3] with a `±0.062` glass.
- Problem, `wmis-native`: 256 bits (the nonce and the generator version). The energy is E = −sum_i w_i x_i + P sum_ij x_i x_j. Here w_i = 2 v_i with v_i in 1 to 99, and P = 200 on every coupler and qubit.
- Solution raw bits: 3,656 (`or-sat-native`), 2,891 (`tait-coloring`), 2,640 (`two-commodity-routing`), 4,089 (`exact-cover-mixed`), 4,569 (`nae3-signed`), 4,575 (`wmis-native`). The compact forms are not measured. The answers are exact covers and Tait colorings (nowhere-zero Z2^2 flows), satisfying assignments, unit flows, not-all-equal assignments (Pauling count about 2^2,300 for `nae3-signed`), or maximum-weight independent sets (no closed form for `wmis-native`).
- Compact formula: yes for the first five cells. An exact MILP proves the optimum of the OR-SAT, Tait, routing and exact-cover cells in 0.15 to 50 s, and the QPU misses it on all 45 models. Not stated for `wmis-native`.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `or-sat-native` | three-literal OR clauses at 0.496 | [`corpora/or-sat-native.jsonl`](../../corpora/or-sat-native.jsonl) | Closed | 1 | Tier 1 |
| `tait-coloring` | honeycomb, 0.248 | [`corpora/tait-coloring.jsonl`](../../corpora/tait-coloring.jsonl) | Closed | 3 | Tier 1 |
| `two-commodity-routing` | street grid, 0.248 | [`corpora/two-commodity-routing.jsonl`](../../corpora/two-commodity-routing.jsonl) | Closed | 4 | Tier 1 |
| `exact-cover-mixed` | factors on K4s, 0.250 gap | [`corpora/exact-cover-mixed.jsonl`](../../corpora/exact-cover-mixed.jsonl) | Closed | 3 | Tier 1 |
| `nae3-signed` | signed triangles, 0.062 glass | [`corpora/nae3-signed.jsonl`](../../corpora/nae3-signed.jsonl) | Closed | 4 | Tier 1 |
| `wmis-native` | 0.050 couplers | [`corpora/wmis-native.jsonl`](../../corpora/wmis-native.jsonl) | Closed | 4 | not recorded |

## Generator and postprocess
The generators are not imported. The research generators are `generate.py` in `families/<cell>` under the new-pow research tree, for each of the six cells (such as `families/wmis-native/generate.py` and `families/exact-cover-mixed/generate.py`). No source file in `sources/` belongs to this family. The production postprocess for all six cells is `constraints` plus `gadget-repair`, with 0.25 s of one core per call for OR-SAT, exact cover and weighted independent set, 0.05 s for Tait and routing, and 0.76 s for not-all-equal triangles. The postprocess code is not imported.

## Results
| Measure | `or-sat-native` | `tait-coloring` | `two-commodity-routing` |
| --- | --- | --- | --- |
| Rank | QPU (0.046 s) < CPU (43.5 s) < CUDA (>101 s) | QPU (0.046 s) < CPU (6.34 s) < CUDA (>132 s) | CPU (0.011 s) < QPU (0.046 s) < CUDA (>50.9 s) |
| Best classical (CPU gap) | CPU 147× (7.2×) | CPU 66× (1.3×) | CPU 0.1× (0.0×) |
| CUDA gap | >342× (>100×) | >1,380× (>164×) | >530× (>63×) |
| Energy gap | 0 (0), QPU 0 / tie 28 / CPU 2, 7.25 s, 30 models | `+1` (1), QPU 0 / tie 0 / CPU 30, 1.06 s, 30 models | 0 (0), QPU 0 / tie 2 / CPU 28, 0.0054 s, 30 models |
| GSE medians (QPU / CUDA / CPU) | `-1,327` / `-1,347` / `-1,359` | `-824` / `-821` / `-831` | `-1,247` / `-1,245` / `-1,253` |
| Deep sweep | >CPU 0 (0) / milp-exact-m30 / 7 s | CPU 2 (1) / 50,000 sweeps / 26 s | >CPU 0 (0) / retime-m30 / <1 s |
| New-pow wins | 12 / 0 / 0 of 12, glasssa, 64 sweeps | 12 / 0 / 0 of 12, coloring anneal (taitsolve) | 12 / 0 / 0 of 12, exact min-cost circulation |
| New-pow wall, ratio vs access | 0.005 s, 0.102 | 0.019 s, 0.414 | 0.004 s, 0.09 |
| Runs (QPU / CUDA / CPU) | 12 / 138 / 301 | 12 / 192 / 345 | 12 / 162 / 252 |

| Measure | `exact-cover-mixed` | `nae3-signed` | `wmis-native` |
| --- | --- | --- | --- |
| Rank | QPU (0.046 s) < CPU (12.3 s) < CUDA (51.8 s) | QPU (0.046 s) < CPU (1.66 s) < CUDA (2.29 s) | QPU (0.046 s) < CUDA (0.517 s) < CPU (1.93 s) |
| Best classical | CPU 42× (2.0×) | CPU 2.1× (1.1×) | CUDA 1.7× (0.5×), CPU gap 6.5× (1.9×) |
| CUDA gap | 175× (51×) | 2.8× (1.5×) | 1.7× (0.5×) |
| Energy gap | `+1` (0), QPU 0 / tie 0 / CPU 30, 2.06 s, 30 models | `+9` (3), QPU 0 / tie 0 / CPU 30, 39.0 s, 30 models | `+7` (1), QPU 0 / tie 0 / CUDA 30, 0.517 s, 30 models |
| GSE medians (QPU / CUDA / CPU) | `-949` / `-952` / `-953` | `-3,193` / `-3,208` / `-3,220` | `-1,941` / `-1,980` / `-1,982` |
| Deep sweep | >CPU 1 (0) / milp-exact / 5 s | CPU 6 (1) / 80,000 sweeps / 4 s | CUDA 7 (1) / 8,192 sweeps / 1 s |
| New-pow wins | 12 / 0 / 0 of 12, pairsa, 512 sweeps | 8 / 1 / 3 of 12, glasssa, 2,048 sweeps | not recorded |
| New-pow wall, ratio vs access | 0.055 s, 1.2 | 0.048 s, 1.04 | not recorded |
| Runs (QPU / CUDA / CPU) | 12 / 168 / 379 | 12 / 236 / 376 | 12 / 42 / 282 |

Notes by cell:
- `or-sat-native`: an exact MILP (`milp-exact-m30`, 7.25 s wall) is the matched CPU rung. The CPU best state meets the postprocessed target at 12 of 12 models. The deepest single CPU run at 1,200 s reaches `-21` energy units below the QPU target on one model.
- `tait-coloring`: the matched CPU rung is a cycle (Kempe chain) campaign, `cyc-depth-2s-m30`, 1.06 s wall. GPU parallel tempering at 4,194,304 sweeps still loses by a median of `+7` energy units.
- `two-commodity-routing`: an exact min-cost circulation closes every model at 0.0054 s wall. The deepest CUDA rung loses by a median of `+1` energy unit on 30 models.
- `exact-cover-mixed`: the GPU single-flip ladder first reaches a majority at 16,384 sweeps (9 of 12 models).
- `nae3-signed`: the CPU lead grows with depth, reaching a median margin of `-28` energy units at 1,200 s on one model.
- `wmis-native`: a compiled greedy (weight over degree plus one, then (ω, 1) and (1, 2) swaps) beats the QPU on 12 of 12 models in a median 0.0126 s on one thread. An iterated local search reaches `-1,981` units in 1,200 s on one model, 97.4 percent of the way from zero to the clique MILP floor of `-1,985` units, where the QPU best stands at 69 percent. The QPU best on that model is `-1,942` units. The greedy verdict improves on it by 23 units (median over models) and the 1,200 s search by 39 units.
- Weak signal: on `wmis-native` analog field error on the QPU is the same size as the weight differences (0.001 to 0.099).

## Limits and tiling
| Cell | Tileable | Reason | Chaining |
| --- | --- | --- | --- |
| `or-sat-native` | unknown | tiling needs zero local fields, and the generator has 3,612 nonzero fields | no, an exact solver closes each link and the QPU does not reach the certified floor |
| `tait-coloring` | no | 11 independent active components across 12 nonce graphs | no, same reason |
| `two-commodity-routing` | unknown | 2,059 nonzero fields | no, same reason |
| `exact-cover-mixed` | no | 1 to 2 independent active components across 12 nonce graphs | no, same reason |
| `nae3-signed` | yes | all three tiled-generator conditions pass across 12 nonces | possible as a hash chain, but simulated annealing at 10,000 sweeps goes 6 to 9 units below the QPU on three models, so a chain would multiply that lead |
| `wmis-native` | unknown | 4,575 nonzero fields | possible, but the CPU greedy beats one call per link (0.013 s against 0.046 s of access), so a chain multiplies the classical lead |

For `nae3-signed` the tiled-generator protocol composes 4 hash-chained tiles in a grid with a 30.0 s budget per tile. Across 12 nonces the tiled and direct median energies are `-12,873.7120` and `-12,869.8920`. The largest absolute delta is 2.418 units per tile, with tiled wins / ties / direct wins of 11 / 0 / 1. Seam strength is 0.062 units and the total satisfied-seam benefit is 0.248 units.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories: `new-pow/or-sat-native`, `new-pow/tait-coloring`, `new-pow/two-commodity-routing`, `new-pow/exact-cover-mixed`, `new-pow/nae3-signed`, `new-pow/wmis-native`, `new-pow/tiled-generator/nae3-signed`, and `new-pow/retime/or-sat-native`, `new-pow/retime/tait-coloring`, `new-pow/retime/two-commodity-routing`, `new-pow/retime/exact-cover-mixed` and `new-pow/retime/nae3-signed`.
