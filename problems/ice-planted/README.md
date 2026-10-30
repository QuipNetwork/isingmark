<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Tile-planted spin ice (F13)

This family plants a known low-energy state in the spin ice tiling. Each tetrahedron carries a gauge-antiferromagnetic K4 pattern that contains the planted state, and extra frustrated cycles are planted in the weak layer. It was screened to see whether a planted optimum with a known energy could stay hidden. It could not. The planted state can be recovered from the public nonce, so the family leaks its own answer.

## Construction
Couplings are in units of the full-strength coupler (J = 1), with the milli value in a code span. The working graph has 4,575 qubits and 41,474 couplers.

- A planted spin vector s is chosen from the nonce.
- Each tetrahedron is a gauge-antiferromagnetic K4 pattern that contains s, at 1 (`1000`).
- Edge-disjoint frustrated triangles or 4-cycles are planted at s* at 0.250 (`250`).
- Fields: h 0. Distinct couplers: {±0.250, ±1}.
- Topology use: 100 percent of qubits and 77.9 percent of couplers.
- Detail: the nonce draws the planted state. Each of the 1,785 tetrahedra takes one of six balanced gauge patterns relative to it, which gives 10,710 strong couplers. The weak layer is a maximal edge-disjoint packing of four-edge cycles on the remaining internal couplers at 0.250, and each weak cycle has one unsatisfied edge in the planted gauge. The generator omits every coupler that no tile covers, so about 82 percent of the eligible internal couplers carry a weak coefficient.
- First nonce: 5,401 weak four-edge cycles, 4,558 of 26,162 eligible internal couplers omitted, and 32,314 couplers in total.
- The construction was expected to fail the planting-leakage test, as the F4 family did.

## Compact representations
- Problem (256 bits: the nonce plus the generator version): gauge-antiferromagnetic K4 tiles plus 0.250 triangles, all planted at s*.
- Solution (4,575 bits, raw at one bit per used qubit, compact size not measured): E0 is the sum of the tile minima, and s* is a ground state.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `ice-planted` | Planted tiles at 1 (`1000`), planted cover at 0.250 (`250`) | [`corpora/ice-planted.jsonl`](../../corpora/ice-planted.jsonl) | Closed | 4 | not measured |

Tier 3 and tier 4 mean that a classical run inside 120 s beats the postprocessed QPU result on at least 60 percent of 30 or more common models. Tier 4 means the best classical run is also within five times the QPU wall time. Tier 1 and tier 2 mean no classical run beats the QPU in that way, and tier 1 means the QPU is also fast. New-pow Tier 1 means classical is strictly lower on a majority of models within 0.1 s. New-pow Tier 2 means parity within 20 min.

## Generator and postprocess
The generator is `qpu-suite/work/ice_planted.py` (research root), which is not imported. Postprocess: the shared `ice-tiles` optimizer ([`icetiles.py`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/icetiles.py), [`icetiles.c`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/icetiles.c), [`method.py`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/method.py)) with `anneal:b0=2,nat48=1` at 0.25 s of one core per call, which gains 0 (0) energy units.

## Results
- Class 1 (classical beats the call cheaply). Profile: classical cheap and CPU-competitive. CPU beats CUDA.
- Attack that closed the family: planting leakage from the public nonce. Public-nonce recovery gives the planted optimum on 24 of 24 models in at most 0.002 s per model.
- Energies listed for this cell: −6,268 energy units for the QPU column, the postprocessed QPU column, the CPU column, and the CUDA column. The QPU best already equals the certified best value on every model, and a median of 36 reads per call sit at the best.
- The suite run rejected the cell without a capture, so QPU against single-flip GPU is not measured. Energy gap: not measured.
- Nonce replay (read the planted state from the public nonce) ties the QPU on all 12 models in a median 0.112 to 0.222 s. The secret-free arm, a gauged cluster anneal with a schedule scaled to the weak layer, ties all 12 models in a median 7.7 s on six threads. A single-flip GPU anneal at 8,192 sweeps ties 30 of 30 models (0.432 card-seconds).
- Best classical: CPU at 0.4 times the QPU access time (0.1 times the QPU wall time). Both cost bounds close the family.
- New-pow campaign: not measured. Verdict date: 2026-10-26.

## Limits and tiling
- Tileable: yes by the tiling assessment. The largest energy delta is 22.25 units per tile, with wins, ties, and losses of 10 / 2 / 0. The tile solvers reached 20 of 48 targets.
- Chaining: no. The planted state is recoverable from the public nonce in about 2 ms per model, and every link of a chain would leak in the same way.
- Limit of the approach: any planted construction whose plant depends only on public data is solved by reading the plant.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories under `/home/carback1/quip-data/regimes`: `qpu-suite/work/cells/ice-planted`, `new-pow/ice-planted`.
