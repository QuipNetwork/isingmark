<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Parity-check gadgets

Parity-check gadgets encode linear checks over GF(2) with auxiliary qubits that sit on native K4,4 blocks. Eliminating the auxiliaries leaves a parity check on a small group of data spins. A weak selector (fields or `±0.062` bonds) chooses among the states that satisfy every check. The set of satisfying states is an affine code coset, which makes the ground state hard to find by local moves but easy for coset-aware solvers. The family was screened as an error-correcting-code style structure. Exact and coset methods close all three cells.

## Construction
Values are in units of the full-strength coupler, with milli units in code spans.

- `four-parity-interacting`: the strong layer is 515 K4,4 gadgets at 0.500 (`500` milli) in the first model. Each gadget joins four private auxiliary qubits to four data qubits. Closed-form elimination of the auxiliaries leaves one four-spin parity check of 1 on the data group. Neighboring checks share data spins. Each data spin sits in two or three checks. Each check touches about four other checks. The weak layer gives every coupler between two data spins an independent sign of `+0.062` or `-0.062` (`±62` milli). This acts as a selector among the states that meet all checks. The nonce seeds the incidence packing, the check bits, the gauges and the selector signs through sha256. The first model has 8,843 nonzero couplers: 8,240 at `±0.500` and 603 at `±0.062`. No qubit carries a field. Couplers are J {`±0.062`, `±0.500`}.
- `parity-k44`: the strong layer is 495 K4,4 gadgets at 0.500. Each gadget joins one vertical group of four private auxiliary qubits to one horizontal group of four data qubits. The coupler between auxiliary a and data spin i is `-0.500` (1 − 2 δ_ai) g_i, where g is a random gauge of four signs. Closed-form elimination of the four auxiliaries leaves one four-spin parity term of 1 on the data group. The gauge product chooses even or odd parity. The weak layer gives every unused coupler between two data spins an independent sign of `+0.062` or `-0.062`. The nonce seeds the gadget placement, the gauges and the selector signs through sha256. The first model has 8,876 nonzero couplers: 7,920 at `±0.500` and 956 at `±0.062`. Fields are 0.
- `ldpc-small-row`: each model packs weight-3 parity checks onto native K4s. A weight-3 check is the gadget 0.500 (x1 + x2 + x3 − 2a − b)^2 on one K4 with three data spins and one private auxiliary spin a. Its Ising terms are `+0.250` between data spins, `-0.500` between each data spin and the auxiliary, and fields of `±0.250` on data spins and `±0.500` on the auxiliary. Four states of each gadget reach the floor, and a data state with the wrong parity costs exactly 0.500 more. A weight-2 check is one data coupler of `-0.250` (`-1`)^b with the same gap and no auxiliary. Every data spin sits in two or three checks, and the check graph is one connected component over the chip. The weak layer is one field of `+0.062` or `-0.062` per data spin, which selects among the states that meet every check. The nonce seeds the packing annealer, the row order, the check bits b and the field signs through framed SHAKE256 draws. The first model has 1,601 checks (1,475 of weight 3 and 126 of weight 2), 1,919 data spins and 8,349 nonzero couplers. Fields span `-0.812` to `+0.812` (16 values) and couplers are J {`±0.250`, `±0.500`, `+0.750`}.

Topology use (qubits / couplers / topology): `four-parity-interacting` 64.7% / 21.4% / 25.7%. `parity-k44` 69.9% / 21.9% / 26.6%. `ldpc-small-row` 76.4% / 20.9% / 26.4%.

## Compact representations
- Problem: 256 bits (the nonce and the generator version). The energy is the gap times the number of violated checks Hx = b. Fields or `±0.062` bonds are added on top. For `parity-k44` the form is `-1`(`-1`)^b prod_{i in c} s_i per check after closed-form elimination, with a `±0.062` selector.
- Solution: raw bits are 2,958 (`four-parity-interacting`), 3,200 (`parity-k44`) and 3,497 (`ldpc-small-row`), one per used qubit. The compact form is not measured. The answer set is an affine coset of size 2^(n − rank H): 2^669 (XZZX), 2^302 to 2^328 (LDPC), 2^371 to 2^386 (four-parity). For `parity-k44` the answer is a product of 8-state checks with tree width 2.
- Compact formula: yes for `four-parity-interacting` and `parity-k44`. Not stated for `ldpc-small-row`.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `four-parity-interacting` | interacting four-spin parity checks, 0.062 selector | [`corpora/four-parity-interacting.jsonl`](../../corpora/four-parity-interacting.jsonl) | Closed | 4 | Tier 1 |
| `parity-k44` | K4,4 parity checks at 0.500, 0.062 selector | [`corpora/parity-k44.jsonl`](../../corpora/parity-k44.jsonl) | Closed | 4 | Tier 1 |
| `ldpc-small-row` | weight-3 LDPC parity checks at 0.500, 0.062 fields | [`corpora/ldpc-small-row.jsonl`](../../corpora/ldpc-small-row.jsonl) | Closed | 3 | not recorded |

## Generator and postprocess
The generators are not imported. No source file in `sources/` belongs to this family. The research generators are `families/four-parity-interacting/generate.py` and `families/ldpc-small-row/generate.py` under the new-pow research tree, and `generate5.py` in the `qpu-f17` research tree for `parity-k44`. The production postprocess is `constraints` plus `gadget-repair`, 0.76 s of one core per call for `four-parity-interacting` and `parity-k44`, and 0.25 s for `ldpc-small-row`. The postprocess code is not imported.

## Results
| Measure | `four-parity-interacting` | `parity-k44` | `ldpc-small-row` |
| --- | --- | --- | --- |
| Rank | CPU (0.012 s) < QPU (0.046 s) < CUDA (>48.1 s) | CPU (0.0075 s) < QPU (0.046 s) < CUDA (>48.1 s) | QPU (0.046 s) < CUDA (>0.401 s) < CPU (1.84 s) |
| Best classical (CPU gap) | CPU 0.0× (0.0×) | CPU 0.0× (0.0×) | CPU 6.2× (1.8×) |
| CUDA gap | >60× (>32×) | >60× (>32×) | >1.4× (>0.4×) |
| Energy gap | `+8` (1), QPU 0 / tie 0 / CPU 30, 1.72 s, 30 models | 0 (0), QPU 0 / tie 5 / CPU 25, 0.0075 s, 30 models | `+8` (2), QPU 0 / tie 0 / CPU 30, 1.84 s, 30 models |
| GSE medians (QPU / CUDA / CPU) | `-2,073` / `-2,072` / `-2,088` | `-1,835` / `-1,826` / `-1,842` | `-1,529` / `-1,539` / `-1,554` |
| Deep sweep | CPU 5 (1) / 50 sweeps / <1 s | >CPU 0 (0) / parity-k44 / <1 s | CPU 8 (2) / `depth-2s-100` / 2 s |
| New-pow wins | 12 / 0 / 0 of 12, GF(2) coset annealer | 12 / 0 / 0 of 12, exact bucket elimination | not recorded |
| New-pow wall, ratio vs access | 0.014 s, 0.301 | 0.005 s, 0.109 | not recorded |
| Runs (QPU / CUDA / CPU) | 12 / 186 / 364 | 12 / 318 / 260 | 12 / 42 / 485 |

The QPU after postprocess reaches `-2,080` (`four-parity-interacting`), `-1,841` (`parity-k44`) and `-1,544` (`ldpc-small-row`) energy units. For `ldpc-small-row`, the 1,200 s state of one model reaches `-1,604` units, inside the family MILP bracket of `-1,605` to `-1,604` units. For `four-parity-interacting`, the deepest full run is CPU 9 (1) with an exact coset solve in 77 s. GPU parallel tempering at 33,554,432 sweeps (413 s) wins 4 of 12 models, with a median margin of `+1` energy unit. For `parity-k44`, GPU parallel tempering at 4,194,304 sweeps still loses all 12 models (median `+9` energy units), and bucket elimination proves the optimum in 0.0094 s per link, faster than one QPU call.

## Limits and tiling
- `four-parity-interacting` is not tileable. Chaining is possible as a hash chain. An exact MILP closes XZZX in 1.6 s per link. LDPC and four-parity are uncaptured, so the per-link ratio is unknown. It is a natural composition target for ice outputs. The tiled-generator run composes 4 hash-chained tiles. Condition 1 fails because a tile interior or the realized global instance has more than one connected component. Condition 2 fails with a largest absolute energy delta of 0.465 units per tile (tiled wins / ties / direct wins 12 / 0 / 0). Condition 3 passes.
- `parity-k44` is not tileable, and it cannot chain. Bucket elimination proves the optimum in 0.0094 s per link, faster than one call, and a fleet of 100 or 1,000 jobs needs the same 0.0095 s per link. Condition 1 fails because the 12 saved nonce graphs have 85 to 97 independent active components before any cut. Condition 2 is unknown, and condition 3 passes.
- `ldpc-small-row` has an unknown tiling status. The tiled-generator protocol is blocked because the generator has 3,394 nonzero fields and tiling needs zero local fields. Chaining is possible, but the CPU beats one call per link. A 1-thread annealer matches all 12 models in a median 0.035 s against 0.046 s of access, so a chain multiplies the classical lead.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories: `regimes/new-pow/four-parity-interacting`, `regimes/new-pow/retime/four-parity-interacting`, `regimes/qpu-f17/work/cells/parity-k44`, `regimes/new-pow/parity-k44`, `regimes/new-pow/ldpc-small-row`, `regimes/new-pow/tiled-generator/four-parity-interacting`.
