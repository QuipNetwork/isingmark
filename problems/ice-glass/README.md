<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Spin ice with a glass, Zephyr pyrochlore (F11)

This family is the base spin ice problem on the Zephyr working graph. About 1,785 corner-sharing K4 tetrahedra carry strong antiferromagnetic bonds. Each tetrahedron has six tied ice states, so the strong layer alone is a large degenerate manifold. A weak random glass on every other coupler breaks the tie. It was screened as the most natural hard case for the QPU, and later families vary the glass, the gauge, or the tile layout. The glass ratio sets the difficulty: the ratio is the strong coupling divided by the glass magnitude.

## Construction
Couplings are in units of the full-strength coupler (J = 1), with the milli value in a code span. The working graph has 4,575 qubits and 41,474 couplers.

- Strong layer: 1,785 corner-sharing K4 tetrahedra. Each tetrahedron is one vertical odd coupler crossing one horizontal odd coupler, plus 4 internal couplers. A nonce-shuffled bipartite matching picks them. All 6 bonds are +1 (`+1000`), which gives 10,710 strong couplers and 6 tied ice states per tetrahedron.
- Weak layer: every other coupler, 30,764 of them, takes a random sign at the glass value.
- Fields: h 0. A nonce-seeded global gauge hides the ice rule.
- Glass values: ±0.125 (`±125`, ratio 8), ±0.200 (`±200`, ratio 5), and ±0.062 (`±62`, ratio 16).
- Topology use: 100 percent of qubits and 100 percent of couplers, with 41,474 nonzero couplers per model.

## Compact representations
- Problem (256 bits: the nonce plus the generator version): `sum_t 0.500[(sum_{i in t} g_ti s_i)^2 - 4]` over 1,785 tiles, with a global gauge (F11) or a per-tile gauge (F12), plus a ±0.031 to ±0.250 glass. F14 adds a ±0.350 middle layer.
- Solution: the strong layer obeys the ice rule with 6 of 16 states per tile. The Pauling count is about 2^1,560. The strong floor of −3,570 is closed form. The glass optimum has no closed form. Size: 4,575 bits raw for `ice-glass-r8` and `ice-glass-r16`, and 2,504 bits compact (mean per qualifying state, at most 2,648) for `ice-glass-r5`.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `ice-glass-r8` | Glass `±125` (ratio 8) | [`corpora/ice-glass-r8.jsonl`](../../corpora/ice-glass-r8.jsonl) | Closed | 4 | Tier 2 |
| `ice-glass-r5` | Glass `±200` (ratio 5) | [`corpora/ice-glass-r5.jsonl`](../../corpora/ice-glass-r5.jsonl) | Closed | 4 | Tier 2 |
| `ice-glass-r16` | Glass `±62` (ratio 16) | [`corpora/ice-glass-r16.jsonl`](../../corpora/ice-glass-r16.jsonl) | Closed | 4 | Tier 1 |

Tier 3 and tier 4 mean that a classical run inside 120 s beats the postprocessed QPU result on at least 60 percent of 30 or more common models. Tier 4 means the best classical run is also within five times the QPU wall time. Tier 1 and tier 2 mean no classical run beats the QPU in that way, and tier 1 means the QPU is also fast. New-pow Tier 1 means classical is strictly lower on a majority of models within 0.1 s. New-pow Tier 2 means parity within 20 min.

## Generator and postprocess
- Structured generator: [`ice_glass.py`](../../tools/structured/isingmark_structured/families/ice_glass.py) ports the suite generator. A random maximum bipartite matching pairs vertical and horizontal odd couplers into tetrahedra with six bonds at `+1000`. Every other coupler takes a protocol draw, and a random physical gauge hides the ice rule. Its `BACKGROUNDS` are 125 and 200, so it covers `ice-glass-r8` and `ice-glass-r5`. The background `62` of `ice-glass-r16` is not in that list.
- Original suite generator: [`ice.py`](../shared/sources/qpu-suite/work/ice.py).
- Generator for `ice-glass-r16`: [`generate2.py`](../twisted-ice/sources/qpu-f17/generate2.py) builds it as untwisted ice with the weak layer at 62 milli.
- Postprocess: the `ice-tiles` optimizer ([`icetiles.py`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/icetiles.py), [`icetiles.c`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/icetiles.c), [`method.py`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/method.py)). The setting is `anneal:fe=64,frac=0.95,nat48=1` at 0.76 s of one core per call for `ice-glass-r8` (gain 4 (3) energy units) and `ice-glass-r16` (gain 17 (3)). For `ice-glass-r5` it is `anneal:b0=4,b1=2,nat48=1` at 0.25 s (gain 1 (1)). Gains are the mean (sample sd) over 12 models against greedy descent.

## Results
| Cell | Class | QPU / QPU pp / CPU / CUDA energy (units) | Energy gap | Best classical | Closed by |
| --- | --- | --- | --- | --- | --- |
| `ice-glass-r8` | 2, classical beats at cost | −4,605 / −4,606 / −4,620 / −4,612 | +6 (4), QPU 9 / tie 1 / CUDA 90, 2.79 s, 100 models | CUDA 3.5× (1.8×) | Fixed-band tempering energy gate at 32,768 sweeps. The cell passes the GPU gate at 23 of 24. |
| `ice-glass-r5` | 1, classical beats cheaply | −5,256 / −5,250 / −5,264 / −5,258 | +5 (5), QPU 1 / tie 0 / CUDA 29, 1.53 s, 30 models | CUDA 5.2× (1.5×) | Single-flip weighted GPU kernel at 131,072 sweeps. |
| `ice-glass-r16` | not recorded | −4,033 / −4,075 / −4,089 / −4,062 | +12 (4), QPU 0 / tie 0 / CPU 30, 32.9 s, 30 models | CPU 1.9× (1.0×) | not recorded |

The best anneal time is 2,000 µs for `ice-glass-r8` and `ice-glass-r5`. For `ice-glass-r16`: not recorded.

`ice-glass-r8`:
- QPU against the best classical attack: tailored fixed-band tempering (`pt-ice-band`) matches 20 of 24 models at 32,768 sweeps for the production call (172.6 one-core seconds per model). At 131,072 sweeps on the strong graph it matches the selected and deep calls (478.3 s per model). The energy gate fails at every captured setting.
- QPU against single-flip GPU: the production call wins 23 of 24 models at 131,072 sweeps and 64 reads (median 57.6 card-seconds per model). It wins 24 of 24 at 8,192 and 32,768 sweeps on the first 12 models. The 2,000 µs setting wins 24 of 24 at 131,072. The median gap at 131,072 sweeps on 12 models is +6,625 milli for the GPU.
- Longer QPU anneals help: the 256-read reference call at 2,000 µs is below the production target on 24 of 24 models (median 7 energy units), and the 48-read call at 2,000 µs on 21 of 24 (median 5).
- Profile: target (QPU leads, CPU competitive). CPU beats CUDA. Strongest attack: 24 / 0 / 0 for fixed-band tempering at 32,768 sweeps, 172.6 s. The GPU at 131,072 sweeps scores 1 / 0 / 23 in 61.3 s.
- New-pow campaign (2026-10-20): 17 / 0 / 6 over 24 models for an Otsuka cluster anneal with a knee schedule at 12,288 sweeps. Median wall 5.22 s, which is 113 times the QPU access time.

`ice-glass-r5`:
- QPU against the best classical attack: the QPU wins 12 of 12 against 6-spin blocks and loop moves at 8,192 sweeps (gap +32,600 at 80 µs), but the GPU gate fails. The GPU beats the production call on 12 of 12 models at 131,072 sweeps.
- Profile: classical cheap and CUDA-bound. CUDA beats CPU. Strongest attack: 12 / 0 / 0 for GPU single flip at 131,072 sweeps, 58.2 s.
- New-pow campaign: 8 / 0 / 4 over 12 models for an Otsuka cluster anneal with a knee schedule at 36,864 sweeps. Median wall 9.2 s, which is 201 times the QPU access time.
- QPU time to target: 0.084 s, from 0.046 s per call and a per-call success rate of 0.55 (median over 12 models, pooled 0.61). Repeat-call spread of the best result: 6 energy units (5 to 10). Rank by time to the postprocessed target: QPU (0.084 s) < CUDA (1.53 s) < CPU (143 s). The 2,000 µs call is below the production target on 12 of 12 models (median 8 energy units).

`ice-glass-r16`:
- Tier 4: the CPU run `depth-30s-100` is below the reference on 30 of 30 models. The CPU run `depth-2s-100` reaches the reference at 1.55 core-seconds per model on 30 models, 1.9 times the QPU access cost. CUDA has no rung that reaches the target on a majority of models. Its deepest rung, a single-flip anneal at 524,288 sweeps, needs more than 38.2 card-seconds (more than 47 times the QPU access cost).
- Rank by time to the postprocessed target: QPU (0.046 s) < CPU (1.55 s) < CUDA (more than 38.2 s).
- New-pow campaign (2026-10-20), Tier 1: 11 / 0 / 1 over 12 models for an Otsuka cluster anneal (`icefast`) at 384 sweeps with population restarts. Median wall 0.053 s, which is 1.15 times the QPU access time.
- The r16 capture is a 48-read call at 80 µs, 0.046 s of access per call. The median read sits 14 energy units above the best read of its call.

Profile and class for r16: not recorded. Verdict date for all three cells: 2026-10-26.

## Limits and tiling
- Coupling scale sets the cost. The verdict CPU wall by glass is 0.053 s for ±0.062 (11 of 12 models match), 5.22 s for ±0.125 (18 of 24), and 9.25 s for ±0.200 (8 of 12). The weaker the glass, the sooner classical cluster moves reach the QPU result.
- Depth at 20 minutes: `ice-glass-r16` reaches a CPU result 55 energy units below the QPU (1 model), `ice-glass-r8` reaches a median 24 below (3 models, deepest 27), and `ice-glass-r5` reaches 30 below (1 model).
- Chaining: a hash chain is possible. Per link, classical matches in 5.2 s on 6 threads for r8 (113 times access) and in 0.082 s for r16. A chain keeps that ratio and adds a round trip per link. Seams between chips would cut tiles and hurt the QPU.
- Tileable: the r8 and r16 cells are tileable. For r8 the largest energy delta is 8.25 units per tile, with wins, ties, and losses of 11 / 0 / 1. The r5 cell is not tileable: condition 2 fails with a largest delta of 10.6 units per tile and tiled wins of 12 / 0 / 0.
- Tiling detail for r16: all three tiled-generator conditions pass across 12 nonces. The largest energy delta is 3.069 units per tile, with tiled wins, ties, and losses of 11 / 0 / 1.
- Repeat-call spread of the QPU for r16: not measured.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories under `/home/carback1/quip-data/regimes`: `qpu-suite/work/cells/ice-glass-r8`, `new-pow/ice-glass-r8`, `qpu-suite/work/cells/ice-glass-r5`, `new-pow/ice-glass-r5`, `qpu-f17/work/cells/ice-glass-r16`, `new-pow/ice-glass-r16`.
