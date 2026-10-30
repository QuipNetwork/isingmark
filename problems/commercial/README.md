<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Commercial benchmark problems

Two industrial scheduling problems form this family. One is sports break minimization (commercial B16). The other is the binary paint shop (commercial B12). Both use one spin per decision. The input fixes the weak, irregular coupling pattern, so no nonce applies. Published inputs fix each instance. The family was screened to see whether real workloads change the picture from the synthetic families. They do not, because a tuned CPU annealer matches or beats the QPU target on both. Energies are in breaks for sports and in color changes for the paint shop.

## Construction
### Sports break minimization (`sports-break`)
Each instance is a published mirrored double round-robin timetable of 4 to 48 teams. The second half of the season repeats the first half with venues swapped. One bit per team pair fixes the home side of the pair's two matches. Every bit assignment is valid. The Ising model counts breaks exactly (two home or two away games in a row for one team).

| Layer | Content |
| --- | --- |
| Strong | couplers of size 1, which are 92,000 of the 94,990 couplers over the 115 models |
| Weak | the 2,990 couplers of size 0.500 |
| Fields | every linear field is 0 |
| Coupling values | `-1`, `-0.500`, `+0.500` and `+1` |
| Example | the 48-team instance `24_v0` has 1,128 spins and 2,256 couplers |

Topology use: 74.8% of qubits (including chains), 11.9% of couplers, 18.1% of the topology.

### Binary paint shop (`paint-shop`)
Each queue holds every car type twice. One spin per car type picks the color of its first car, and the second car takes the other color, so the Ising model counts the color changes along the queue. The register fixes the three input queues, so no nonce sets the structure. Every field is 0.

| Queue | Weak couplers (`+0.500` and `-0.500`) | Strong couplers (`-1`) | Offset |
| --- | --- | --- | --- |
| 128 types | 250 (85 at `+0.500` and 165 at `-0.500`) | 1 | 128 units |
| 256 types | 504 | 3 | 256 units |
| 512 types | 1,021 | 1 | 512 units |

Topology use: 64.3% of qubits (including chains), 8.3% of couplers, 13.9% of the topology.

## Compact representations
- Problem, sports: 8 bits (instance id: a 5-bit team count of 23 sizes plus a 3-bit variant of 5 in the published register). Breaks = sum J_ij s_i s_j + sum h_i s_i + offset, over one spin per team pair.
- Solution, sports: 1,128 bits raw, one per team pair at 48 teams. The answer is the schedule with the fewest breaks. No closed form exists, and the de Werra bound (138 at 48 teams) is far below the solver floor. On the 4-team instance `02_v0` the code is 6 bits against 6 raw bits, so a small instance has no compact gain.
- Problem, paint shop: 9,216 bits (the car sequence of the largest queue: 1,024 cars × 9 bits per car type). Color changes = sum J_ij s_i s_j + sum h_i s_i + offset, over one spin per car type.
- The paint-shop solution is 512 bits raw, one per type at 512 types. The answer is the coloring with the fewest changes. No closed form exists, and HiGHS proves the optimum only on the 128-type queue.

## Variants
| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `sports-break` | 115 timetables of 4 to 48 teams | none (own small graph, not on chain-h0) | Closed | 3 | Tier 1 |
| `paint-shop` | 3 queues of 128, 256 and 512 car types | none (own small graph, not on chain-h0) | Closed | 4 | Tier 1 |

## Generator and postprocess
No generator exists. The inputs are published timetables (the source of the 115 timetables and of the one-bit-per-pair break QUBO is Kuramata, Katsuki and Nakata, 2022, PLOS ONE 17(4), e0266846) and three paint-shop queues. The build code is not imported. No source file in `sources/` belongs to this family. The postprocess is natural-move descent on the stored reads: 48 reads per sports instance at 80 µs (median 0.0089 s per call) and 256 reads per paint-shop queue (median 0.017 s per call). The postprocess code is not imported.

## Results
No GPU ran on these workloads, so CUDA values are not measured.

| Measure | `sports-break` | `paint-shop` |
| --- | --- | --- |
| Rank | QPU (0.045 s) < CPU (0.132 s) | CPU (0.024 s) < QPU (0.094 s) |
| Best classical (CPU gap) | CPU 1.7× (0.0×) | CPU 0.1× (0.0×) |
| QPU TTS | 0.045 s (48 reads at 80 µs, one call per instance) | 0.094 s (256 reads at 80 µs, one call per queue) |
| GSE medians (QPU / CPU) | 560 / 542 breaks (12 depth instances) | 69 / 68 color changes (3 depth queues) |
| QPU after postprocess | 180 breaks | 69 color changes |
| Verdict rung | `full-s1000-a48`: 115 of 115 instances at or below the QPU, 0.022 s on 6 threads | `screen-s100-a48`: 3 of 3 queues at or below the QPU, 0.004 s on 6 threads |
| Median margin at verdict | `-2.0` breaks | `-1.0` color changes |
| Deepest CPU result, 12 depth instances | median `-18.0` breaks | median `-1.0` color changes |
| 1,200 s result | `-24.0` breaks on 1 instance | `-8.0` color changes on 1 queue |
| Deep sweep | >CPU 4 (6) / full-s2000-a48 / <1 s | >CPU 3 (4) / rep-s100-a48-seed3 / <1 s |
| New-pow wins | 58 / 57 / 0 of 115 instances, glasssa, 1,000 sweeps | 2 / 1 / 0 of 3 queues, glasssa, 100 sweeps |
| New-pow wall, ratio vs access | 0.019 s, 0.413 | 0.004 s, 0.087 |
| Runs (QPU / CUDA / CPU) | 115 / 0 / 2,041 | 3 / 0 / 124 |

On the sports instance `02_v0`, the CPU best state gives 6 breaks. That equals the QPU best, the proven best value and the de Werra bound. On the paint-shop queue `paint-128-10000`, the verdict state has 37 color changes, the proven best value.

## Limits and tiling
The tiling status is unknown for both cells. The tiling protocol is blocked because the construction has no nonce generator. The QPU call-to-call spread is not measured, because each instance has one call. A classical link takes 0.025 s (sports) or 0.005 s (paint shop). A QPU call takes 0.1 s, so a chain amplifies the classical lead.

## Provenance
Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories: `new-pow/commercial/sports-break`, `new-pow/commercial/paint-shop`.
