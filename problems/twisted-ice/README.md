<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Twisted spin ice (F12)

Twisted spin ice puts a K4 tile on every tetrahedron of the working Zephyr graph. Each tile has its own random gauge, so qubits shared by two tiles see two gauges. The strong layer no longer matches plain spin ice under a gauge change. A weak random glass of fixed size fills every other coupler. The screen chose this family because the strong layer is a signed ice rule with a closed-form floor, while the glass on top has no closed form. The 25 variants change the glass strength, the planted pair count and strength, and the chip window. One variant plants 4-loops.

## Construction

All variants share one base construction. The base cell is `ice-twisted`.

- Strong layer: 1,785 K4 tiles. Tile t has a gauge g_t in `{+1, -1}^4`. Its six bonds are `+1 g_ta g_tb`, so each bond is `+1` or `-1` in units of the full-strength coupler (milli units `1000` and `-1000`). A global sign flip leaves the interior energy unchanged.
- Weak layer: every other coupler is +w or -w, with random signs. The base cell uses w = 0.125 (`125` milli). The variants use 0.031, 0.045, 0.062, 0.090, and 0.250 (`31`, `45`, `62`, `90`, `250` milli).
- Fields: none. Every qubit has h = 0.
- Seeding: the nonce fixes the tile matching and the gauge of each tile. It also fixes the glass signs. The suite generator derives all three from a sha256 hash of the nonce.
- Topology use of the base cell: 100 percent of the 4,575 working qubits and 100 percent of the 41,474 working couplers (46,049 of 46,049 in total). A model has 10,710 strong couplers at +`-1` and 30,764 weak couplers at +`-0.125`.

Planted pairs. A greedy matching adds P pair bonds at `-1` (`-1000` milli) on couplers outside the tiles. The matching comes from `sha256("dimer" || nonce)`. Pairs can share spins with tiles. The recipe runs in this order:

1. Take the base `ice-twisted` model of the nonce.
2. Place the pairs on couplers with |J| = 0.125, skipping a pair if either endpoint is used. The |J| = 0.125 filter keeps pairs off tile bonds, which have size 1.
3. Rescale every remaining |J| = 0.125 coupler to the weak value (`62` milli at glass 0.062).

Step 2 must come before step 3, because no |J| = 0.125 coupler remains after the rescale. With 1,200 pairs and w = 0.062, a model has 11,910 strong couplers (10,710 tile bonds and 1,200 pairs) and 29,564 weak couplers, 41,474 in all.

Half-strength pairs (`ice-twisted-w62-dimer1200-p500`). The 1,200 pairs sit at `-0.500` (`-500` milli) instead of `-1`. Breaking one pair costs 1 unit. A model has 10,710 couplers at +`-1`, 1,200 at `-0.500` and 29,564 at +`-0.062`.

Planted 4-loops (`ice-twisted-w62-loop4x300`). The cell plants 300 disjoint rings of four `-1` bonds on couplers outside the tiles, 1,200 ring bonds in all. Each ring ties four qubits in four different tiles. A model has 11,910 strong couplers (10,710 tile bonds and 1,200 ring bonds) and 29,564 weak couplers at +`-0.062`.

Windows. A window cell cuts a square region of the chip and keeps the tiles inside it. The window `L9` of the 1,200-pair cell has 547 spins and 165 tiles, and 132 to 144 planted pairs. The first model has 1,122 strong couplers (467 at `+1` and 655 at `-1`) and 3,301 weak couplers (1,681 at `+0.062` and 1,620 at `-0.062`). The window uses 12 percent of the qubits (547 of 4,575), 10.7 percent of the couplers (4,423 of 41,474), and 10.8 percent of the topology (4,970 of 46,049).

Cell sizes. Strong couplers are tile bonds, planted pairs, and ring bonds. Weak couplers are the glass. "Used" is the share of the 4,575 working qubits and 41,474 working couplers that carry a nonzero value in the first model.

| Cell | Strong couplers | Weak couplers | Used qubits, couplers |
| --- | --- | --- | --- |
| `ice-twisted`, `-w31`, `-w45`, `-w62`, `-w90`, `-w250` | 10,710 | 30,764 | 100 percent, 100 percent |
| `ice-twisted-dimer`, `-w62-dimer`, `-w90-dimer` (600 pairs) | 11,310 | 30,164 | 100 percent, 100 percent |
| `ice-twisted-dimer1200`, `-w45-dimer1200`, `-w62-dimer1200`, `-w90-dimer1200`, `-w62-loop4x300` | 11,910 | 29,564 | 100 percent, 100 percent |
| `ice-twisted-w62-dimer1350` | 12,060 | 29,414 | 100 percent, 100 percent |
| `ice-twisted-w62-dimer1500` | 12,210 | 29,264 | 100 percent, 100 percent |
| `ice-twisted-w62-dimer1800` | 12,510 | 28,964 | 100 percent, 100 percent |
| `ice-twisted-w62-dimer1200-p500` | 10,710 at `±1` and 1,200 at `-0.500` | 29,564 | 100 percent, 100 percent |
| `ice-twisted-L9` | 990 | 3,433 | 12 percent (547 spins), 10.7 percent |
| `ice-twisted-L13` | 2,388 | 7,563 | 25.9 percent (1,184 spins), 24 percent |
| `ice-twisted-L18` | 5,202 | 15,292 | 51 percent (2,335 spins), 49.4 percent |
| `ice-twisted-w62-dimer1200-L9` | 1,122 | 3,301 | 12 percent (547 spins), 10.7 percent |
| `ice-twisted-w62-dimer1200-L13` | 2,678 | 7,273 | 25.9 percent (1,184 spins), 24 percent |
| `ice-twisted-w62-dimer1200-L18` | 5,787 | 14,707 | 51 percent (2,335 spins), 49.4 percent |
| `ice-twisted-w62-dimer1200-L22` | 8,971 | 22,302 | 77 percent (3,521 spins), 75.4 percent |

Window sizes in tiles are 165 for L9, 398 for L13, 867 for L18, and 1,344 for L22. The L13 window of the 1,200-pair cell keeps 290 planted pairs. The L9 window keeps 132 to 144 pairs.

Two cells have extra structure. At glass `0.250` the best known states leave the ice manifold. They have a strong energy near `-3,390` against the ice floor of `-3,570`, with about 90 defect tiles. At 1,500 pairs the strong layer cannot be fully satisfied, and the best states hold 8 to 17 defective tiles and 10 to 19 broken pairs. At 1,800 pairs the strong layer is unsatisfiable, and every low state has 22 to 29 defective tiles and 19 to 36 broken pairs.

## Compact representations

Problem (all variants). Serialized size is 256 bits (the nonce plus the generator version). The Hamiltonian is

```txt
sum_t 0.500[(sum_{i in t} g_ti s_i)^2 - 4]   over 1,785 tiles, per-tile gauge g_t
+ weak glass of +-w on every other coupler
+ sum_pairs -1 s_a s_b                       (P = 600 to 1,800 planted pairs, where present)
```

The related F11 family uses one global gauge. The `p500` cell scales the pair term to `-0.500 s_a s_b`. The loop cell replaces the pair term with `sum_rings sum_{ab in ring} -1 s_a s_b` over 300 four-bond rings. A window cell uses the parent formula on a square window with boundary tiles dropped.

The explicit form of the largest model is 41,474 couplers at 2 bits each, 82,948 bits, because 4 coupler values occur. With pairs at two strengths, 5 values occur and it is 41,474 couplers at 3 bits each, 124,422 bits.

Solution.

- Base cell: the strong layer obeys the ice rule, with 6 of 16 states per tile. The Pauling count is about 2^1,560. The strong floor is `-3,570` units in closed form and a bidirected b-matching, which an integer solver reaches in 0.04 to 0.08 s. The glass optimum has no closed form. The compact size is 2,221 bits mean and 2,315 bits at most over 310 qualifying states, against 4,575 bits raw. About 470 bits of the mean are free spins and about 36 bits are header. A worked first-call best codes to 2,235 bits, with 3 tile defects, 0 pair defects, and 471 free spins.
- Pair cells: the answer set is the ice states with every planted pair equal. The count is about 2^(1,560 - P), which reaches zero near 1,560 pairs. The integer-solver time agrees: 0.06 s at 1,200 pairs and over 120 s at 1,500 pairs. For `ice-twisted-w62-dimer1200` the compact size is 1,432 bits mean and 1,529 bits at most over 246 qualifying states. The first-call best codes to 1,405 bits, with 5 tile defects, 5 pair defects, and 471 free spins.
- Loop cell: ice states with every ring equal, about 900 independent equalities. The compact size is not measured. Raw is 4,575 bits.
- Window cell: the ice-rule answer set on the kept tiles. For `L9` of the 1,200-pair cell the raw size is 547 bits (one per used qubit). The compact size is not measured.

## Variants

The tier rule counts a beat when a classical run within 120 s is strictly below the postprocessed QPU energy on 60 percent of 30 or more common models. Tier 1 and 2 mean no beat. Tier 1 applies when the QPU is fast, and tier 2 otherwise. Tier 3 and 4 mean a beat. Tier 3 applies when the QPU is fast, and tier 4 otherwise. The new-pow verdict follows the `rescore.py` rule. Tier 1 means classical is strictly lower on a majority of models within 0.1 s. Tier 2 means parity within 20 min. Every cell has status Closed. Each corpus link names the file for the first-nonce model of the cell.

| Cell | Parameters | Example corpus | Status | Tier | New-pow verdict |
| --- | --- | --- | --- | --- | --- |
| `ice-twisted` | glass 0.125, no pairs | [`corpora/ice-twisted.jsonl`](../../corpora/ice-twisted.jsonl) | Closed | 4 | Tier 2 |
| `ice-twisted-w62-dimer1200-L13` | glass 0.062, 1,200-pair cell, window L13 | [`corpora/ice-twisted-w62-dimer1200-L13.jsonl`](../../corpora/ice-twisted-w62-dimer1200-L13.jsonl) | Closed | 3 | Tier 2 |
| `ice-twisted-L9` | glass 0.125, window L9 | [`corpora/ice-twisted-L9.jsonl`](../../corpora/ice-twisted-L9.jsonl) | Closed | 4 | Tier 1 |
| `ice-twisted-dimer` | glass 0.125, 600 planted pairs | [`corpora/ice-twisted-dimer.jsonl`](../../corpora/ice-twisted-dimer.jsonl) | Closed | 3 | Tier 2 |
| `ice-twisted-w250` | glass 0.250, no pairs | [`corpora/ice-twisted-w250.jsonl`](../../corpora/ice-twisted-w250.jsonl) | Closed | 4 | Tier 2 |
| `ice-twisted-w90-dimer1200` | glass 0.090, 1,200 planted pairs | [`corpora/ice-twisted-w90-dimer1200.jsonl`](../../corpora/ice-twisted-w90-dimer1200.jsonl) | Closed | 3 | Tier 2 |
| `ice-twisted-w62-dimer1350` | glass 0.062, 1,350 planted pairs | [`corpora/ice-twisted-w62-dimer1350.jsonl`](../../corpora/ice-twisted-w62-dimer1350.jsonl) | Closed | 1 | Tier 2 |
| `ice-twisted-w62-dimer1500` | glass 0.062, 1,500 planted pairs | [`corpora/ice-twisted-w62-dimer1500.jsonl`](../../corpora/ice-twisted-w62-dimer1500.jsonl) | Closed | 3 | Tier 2 |
| `ice-twisted-w62-dimer` | glass 0.062, 600 planted pairs | [`corpora/ice-twisted-w62-dimer.jsonl`](../../corpora/ice-twisted-w62-dimer.jsonl) | Closed | 4 | Tier 2 |
| `ice-twisted-w31` | glass 0.031, no pairs | [`corpora/ice-twisted-w31.jsonl`](../../corpora/ice-twisted-w31.jsonl) | Closed | 4 | Tier 1 |
| `ice-twisted-w62-loop4x300` | glass 0.062, 300 planted 4-loops (1,200 ring bonds at `-1`) | [`corpora/ice-twisted-w62-loop4x300.jsonl`](../../corpora/ice-twisted-w62-loop4x300.jsonl) | Closed | 3 | Tier 2 |
| `ice-twisted-w45` | glass 0.045, no pairs | [`corpora/ice-twisted-w45.jsonl`](../../corpora/ice-twisted-w45.jsonl) | Closed | 4 | Tier 1 |
| `ice-twisted-w62-dimer1800` | glass 0.062, 1,800 planted pairs | [`corpora/ice-twisted-w62-dimer1800.jsonl`](../../corpora/ice-twisted-w62-dimer1800.jsonl) | Closed | 3 | Tier 2 |
| `ice-twisted-w62` | glass 0.062, no pairs | [`corpora/ice-twisted-w62.jsonl`](../../corpora/ice-twisted-w62.jsonl) | Closed | 3 | Tier 1 |
| `ice-twisted-w62-dimer1200-L22` | glass 0.062, 1,200-pair cell, window L22 | [`corpora/ice-twisted-w62-dimer1200-L22.jsonl`](../../corpora/ice-twisted-w62-dimer1200-L22.jsonl) | Closed | 1 | Tier 2 |
| `ice-twisted-w62-dimer1200-L9` | glass 0.062, 1,200-pair cell, window L9 | [`corpora/ice-twisted-w62-dimer1200-L9.jsonl`](../../corpora/ice-twisted-w62-dimer1200-L9.jsonl) | Closed | 1 | Tier 2 |
| `ice-twisted-w62-dimer1200-L18` | glass 0.062, 1,200-pair cell, window L18 | [`corpora/ice-twisted-w62-dimer1200-L18.jsonl`](../../corpora/ice-twisted-w62-dimer1200-L18.jsonl) | Closed | 3 | Tier 2 |
| `ice-twisted-dimer1200` | glass 0.125, 1,200 planted pairs | [`corpora/ice-twisted-dimer1200.jsonl`](../../corpora/ice-twisted-dimer1200.jsonl) | Closed | 3 | Tier 2 |
| `ice-twisted-L18` | glass 0.125, window L18 | [`corpora/ice-twisted-L18.jsonl`](../../corpora/ice-twisted-L18.jsonl) | Closed | 3 | Tier 2 |
| `ice-twisted-w62-dimer1200-p500` | glass 0.062, 1,200 half-strength pairs (J = `-0.500`) | [`corpora/ice-twisted-w62-dimer1200-p500.jsonl`](../../corpora/ice-twisted-w62-dimer1200-p500.jsonl) | Closed | 3 | Tier 2 |
| `ice-twisted-w45-dimer1200` | glass 0.045, 1,200 planted pairs | [`corpora/ice-twisted-w45-dimer1200.jsonl`](../../corpora/ice-twisted-w45-dimer1200.jsonl) | Closed | 1 | Tier 2 |
| `ice-twisted-w62-dimer1200` | glass 0.062, 1,200 planted pairs | [`corpora/ice-twisted-w62-dimer1200.jsonl`](../../corpora/ice-twisted-w62-dimer1200.jsonl) | Closed | 1 | Tier 2 |
| `ice-twisted-L13` | glass 0.125, window L13 | [`corpora/ice-twisted-L13.jsonl`](../../corpora/ice-twisted-L13.jsonl) | Closed | 4 | Tier 2 |
| `ice-twisted-w90` | glass 0.090, no pairs | [`corpora/ice-twisted-w90.jsonl`](../../corpora/ice-twisted-w90.jsonl) | Closed | 3 | Tier 2 |
| `ice-twisted-w90-dimer` | glass 0.090, 600 planted pairs | [`corpora/ice-twisted-w90-dimer.jsonl`](../../corpora/ice-twisted-w90-dimer.jsonl) | Closed | 3 | Tier 2 |

Glass strengths are in units of the full-strength coupler. Pair strength is `-1` unless stated.

## Generator and postprocess

Instance generation:

- Base cell: [`sources/qpu-suite/work/ice_twisted.py`](sources/qpu-suite/work/ice_twisted.py) builds the base twisted ice model (`generate_model`, with a NumPy `default_rng` seeded from the sha256 hash of the nonce). The tile geometry comes from [`../shared/sources/qpu-suite/work/ice.py`](../shared/sources/qpu-suite/work/ice.py). The isingmark generator is [`tools/structured/isingmark_structured/families/ice_twisted.py`](../../tools/structured/isingmark_structured/families/ice_twisted.py), with parameters in [`params/ice-twisted.json`](params/ice-twisted.json).
- Pairs, loops, and weak rescale: [`sources/qpu-f17/generate2.py`](sources/qpu-f17/generate2.py) holds `add_dimers`, `add_loops`, `rescale_weak`, `rng_for`, and a `VARIANTS` table. The table holds only the first recipe, `rescale_weak(add_dimers(m, n, 1200), 62)`.
- Recipe driver: [`sources/qpu-f17/wave_run.py`](sources/qpu-f17/wave_run.py) sets the pair count and weak value (`--pairs`, `--weak`) and the pair strength (`--pair-milli`). Examples: `--pairs 1350 --weak 62`, `--pairs 1200 --weak 45`, and `build --pairs 1200 --weak 62 --pair-milli -500` for the half-strength cell. [`../shared/sources/new-pow/models-100/gen100.py`](../shared/sources/new-pow/models-100/gen100.py) (`wave`, `twisted_base`, `nonce_for`) extends each cell to 100 nonces.
- Adversary targets: [`sources/qpu-f17/adversary/targets.json`](sources/qpu-f17/adversary/targets.json).
- Window cells: the code that cuts the square windows is not imported. The window models exist in the research cell directories named under Provenance.
- Nonce cohorts: the master list has 24 nonces made by `default_rng(20260920)`. The screen cohort is the first 12. Extension nonces come from `sha256(b"new-pow/models-100/" + str(i))`.

Postprocess. Each cell has one postprocess that runs on the 48 reads of the production call. Code for `ice-tiles`: [`../shared/sources/new-pow/postprocess/optimizers/ice-tiles/`](../shared/sources/new-pow/postprocess/optimizers/ice-tiles/). Code for `ice-pairs`: [`sources/new-pow/postprocess/optimizers/ice-pairs/`](sources/new-pow/postprocess/optimizers/ice-pairs/) (`pairs.py`, `pairworm.c`, `run.py`, `Makefile`). Shared descent helpers: [`../shared/sources/new-pow/postprocess/greedy.c`](../shared/sources/new-pow/postprocess/greedy.c) and [`../shared/sources/new-pow/postprocess/descent.py`](../shared/sources/new-pow/postprocess/descent.py).

| Cells | Postprocess | Budget per call |
| --- | --- | --- |
| `ice-twisted`, `ice-twisted-L9`, `-L13`, `-L18` | `ice-tiles`, `anneal:b0=2,nat48=1` | 0.25 s of one core |
| `ice-twisted-w250` | `ice-tiles`, `anneal:b0=4,b1=2,nat48=1` | 0.25 s |
| `ice-twisted-w62-loop4x300` | `ice-tiles`, `anneal:fe=64,frac=0.95,nat48=1` | 0.25 s |
| `ice-twisted-w31`, `-w45`, `-w62`, `-w90` | `ice-tiles`, `anneal:fe=64,frac=0.95,nat48=1` | 0.76 s |
| `ice-twisted-dimer`, `ice-twisted-w90-dimer1200`, `ice-twisted-w62-dimer1200-L9`, `-L13` | `ice-pairs`, method `pw` | 0.25 s |
| `ice-twisted-dimer1200`, `-w45-dimer1200`, `-w62-dimer`, `-w62-dimer1200`, `-w62-dimer1350`, `-w62-dimer1500`, `-w62-dimer1800`, `-w62-dimer1200-p500`, `-w62-dimer1200-L18`, `-w62-dimer1200-L22`, `-w90-dimer` | `ice-pairs`, method `pw` | 0.76 s |

Strongest classical attack code (CPU fork, representative tree): [`../shared/sources/new-pow/ice-twisted-w62-dimer1200/cpu/`](../shared/sources/new-pow/ice-twisted-w62-dimer1200/cpu/). The `pair_bench.rs` example holds the Otsuka cluster anneal with soft pair bonds. The loop, `L18`, `L22`, and `p500` variants of that file are not imported. The `l9_fast.rs` example, used for the `L9` and `L13` windows, is not imported.

## Results

QPU reference: one production call of 48 reads at 80 µs, 0.046 s of charged access, followed by a per-family postprocess. Energy units are whole units of one full-strength coupler. The energy gap is the mean (sample sd) of the postprocessed QPU energy minus the classical energy. A positive value means classical is deeper. Counts give strict QPU wins, ties, and classical wins at the deciding run, then median wall per model and the number of common models. The access ratio is the classical device time over 0.046 s of QPU access plus postprocess. The wall ratio is in parentheses. The best classical device is the one with the smaller access ratio.

Base cell `ice-twisted` has class 2 (classical beats at cost) and a target profile (QPU leads, CPU competitive). CPU beats CUDA. Results:

- QPU energy, median over 24 models: `-4,607` (lowest read) and `-4,605` after the `ice-tiles` postprocess (median over 12 models). CUDA reaches `-4,609` and CPU reaches `-4,619` at their best runs.
- Closed by: fixed-band tempering on the full graph at 131,072 sweeps, 494.6 one-core seconds per model, which leaves the production call and the selected anneal with zero QPU wins among 24 models. The deep reference (256 reads at 2,000 µs) keeps 3 wins. The single-flip GPU kernel at 131,072 sweeps and 64 reads loses to the production call on 21 of 24 models.
- Time to target. The QPU needs 0.046 s, CUDA needs 1.38 card-seconds, and the CPU needs 122 core-seconds, in rank order QPU < CUDA < CPU. The cheapest CUDA rung is the Otsuka cluster anneal at 16,384 sweeps (4.7x access, 1.4x wall). The cheapest CPU rung is the 120 s 100-model rung (411x access, 121x wall).
- New-pow campaign: the Otsuka cluster anneal at 32,768 sweeps wins on 24 of 24 models at 11.8 s wall per model, 257 times the QPU access. The CPU depth ladder at 6 threads reaches the target on 24 of 24 models at 11.8 s wall (71.1 core-seconds), 19 of 24 at 8.80 s, and 0 of 24 at 2.57 s. The 20-minute rung reaches a median of `-21` units below the QPU target (deepest `-24`, 3 models). QPU calls with the deepest anneal reach a median of `-3` units.
- QPU anneal captures: 48 reads at 200, 500, and 2,000 µs beat the production target on 6 of 12, 7 of 12, and 18 of 24 models. 256 reads at 2,000 µs beat it on 24 of 24. Repeat-call spread is not measured.
- Seed spread: the verdict CPU kernel matches 79 of 96 model-runs over four runs. The GPU cluster anneal at 8,192 sweeps matches 23 of 24 and 21 of 24 models over two seeds.

The tables below group the other variants. Only the base cell has a recorded class and profile.

### Glass strength, no pairs

| Cell | Best classical, access ratio (wall ratio) | Energy gap | New-pow wins / ties / losses, solver | New-pow wall, s | Tileable |
| --- | --- | --- | --- | --- | --- |
| `ice-twisted-w31` | CPU 2.4× (1.3×) | `+9 (2), QPU 0 / tie 0 / CPU 30, 33.3 s, 30 models` | 12 / 0 / 0 of 12, Otsuka cluster anneal (icefast), 32 sweeps, and worm polish | 0.011 | No |
| `ice-twisted-w45` | CPU 2.2× (1.2×) | `+13 (4), QPU 0 / tie 0 / CPU 30, 32.2 s, 30 models` | 12 / 0 / 0 of 12, Otsuka cluster anneal (icefast), 32 sweeps, and worm polish | 0.011 | Yes |
| `ice-twisted-w62` | CPU 10× (5.5×) | `+10 (4), QPU 0 / tie 0 / CPU 30, 38.4 s, 30 models` | 11 / 0 / 1 of 12, Otsuka cluster anneal (icefast), 384 sweeps with population restarts | 0.053 | No |
| `ice-twisted-w90` | CPU 38× (20×) | `+10 (4), QPU 0 / tie 0 / CPU 30, 136 s, 30 models` | 8 / 0 / 4 of 12, Otsuka cluster anneal (icefast), 4,096 sweeps | 0.879 | No |
| `ice-twisted` | CUDA 4.7× (1.4×) | `+6 (4), QPU 1 / tie 0 / CUDA 29, 2.76 s, 30 models` | 24 / 0 / 0 of 24, Otsuka cluster anneal, 32,768 sweeps | 11.8 | Yes |
| `ice-twisted-w250` | CUDA 5.0× (1.5×) | `+5 (5), QPU 2 / tie 0 / CUDA 28, 1.48 s, 30 models` | 10 / 0 / 2 of 12, Otsuka cluster anneal (icefast), 32,768 sweeps | 10.9 | Yes |

### Planted pairs at glass 0.125

| Cell | Best classical, access ratio (wall ratio) | Energy gap | New-pow wins / ties / losses, solver | New-pow wall, s | Tileable |
| --- | --- | --- | --- | --- | --- |
| `ice-twisted-dimer` | CUDA 6.7× (2.0×) | `+6 (4), QPU 2 / tie 1 / CUDA 27, 3.23 s, 30 models` | 6 / 1 / 5 of 12, population-annealing cluster kernel, 12,288 sweeps | 5.48 | No |
| `ice-twisted-dimer1200` | CUDA 5.7× (3.0×) | `+10 (5), QPU 1 / tie 0 / CUDA 29, 9.19 s, 30 models` | 12 / 0 / 0 of 12, Otsuka cluster anneal with soft pair bonds, 49,152 sweeps | 14.6 | No |

### Planted pairs at glass 0.090

| Cell | Best classical, access ratio (wall ratio) | Energy gap | New-pow wins / ties / losses, solver | New-pow wall, s | Tileable |
| --- | --- | --- | --- | --- | --- |
| `ice-twisted-w90-dimer` | CUDA 15× (7.8×) | `+11 (3), QPU 0 / tie 0 / CUDA 100, 11.8 s, 100 models` | 8 / 0 / 4 of 12, population-annealing cluster kernel, 12,288 sweeps | 5.54 | No |
| `ice-twisted-w90-dimer1200` | CUDA 24× (6.5×) | `+7 (4), QPU 6 / tie 0 / CUDA 94, 7.07 s, 100 models` | 9 / 0 / 3 of 12, Otsuka cluster anneal, knee schedule, 49,152 sweeps | 12.5 | No |

### Planted pairs at glass 0.062 (pair-count scan)

| Cell | Best classical, access ratio (wall ratio) | Energy gap | New-pow wins / ties / losses, solver | New-pow wall, s | Tileable |
| --- | --- | --- | --- | --- | --- |
| `ice-twisted-w62-dimer` | CUDA 4.2× (2.2×) | `+7 (4), QPU 1 / tie 0 / CPU 29, 133 s, 30 models` | 8 / 0 / 4 of 12, Otsuka cluster anneal with population resampling, 4,096 sweeps | 1.66 | Yes |
| `ice-twisted-w62-dimer1200` | CPU 390× (198×) | `-4 (6), QPU 22 / tie 0 / CPU 8, 122 s, 30 models` | 23 / 0 / 1 of 24, Otsuka cluster anneal, tile weight 250, 65,536 sweeps | 27 | Yes |
| `ice-twisted-w62-dimer1350` | CPU 675× (358×) | `-1 (5), QPU 19 / tie 0 / CPU 11, 157 s, 30 models` | 22 / 0 / 2 of 24, Otsuka cluster anneal, tile weight 250, 49,152 sweeps | 17 | No |
| `ice-twisted-w62-dimer1500` | CUDA 18× (9.4×) | `+7 (5), QPU 9 / tie 0 / CUDA 91, 14.3 s, 100 models` | 10 / 0 / 2 of 12, population annealing with pair and tetrahedron moves, 32,768 sweeps | 12.5 | No |
| `ice-twisted-w62-dimer1800` | CUDA 19× (9.8×) | `+8 (5), QPU 4 / tie 0 / CUDA 96, 14.9 s, 100 models` | 24 / 0 / 0 of 24, single-flip anneal with pair and tetrahedron moves, 131,072 sweeps | 13.8 | No |

### Planted pairs at glass 0.045, half-strength pairs, 4-loops

| Cell | Best classical, access ratio (wall ratio) | Energy gap | New-pow wins / ties / losses, solver | New-pow wall, s | Tileable |
| --- | --- | --- | --- | --- | --- |
| `ice-twisted-w45-dimer1200` | CPU 542× (288×) | `-2 (4), QPU 19 / tie 0 / CPU 11, 229 s, 30 models` | 12 / 0 / 0 of 12, Otsuka cluster anneal, knee schedule, 20,480 sweeps | 8.31 | No |
| `ice-twisted-w62-dimer1200-p500` | CUDA 8.5× (4.5×) | `+7 (3), QPU 1 / tie 0 / CUDA 99, 13.5 s, 100 models` | 11 / 0 / 1 of 12, Otsuka cluster anneal, knee schedule, 12,288 sweeps | 6.85 | No |
| `ice-twisted-w62-loop4x300` | CUDA 8.6× (2.5×) | `+10 (5), QPU 0 / tie 0 / CPU 30, 120 s, 30 models` | 11 / 0 / 1 of 12, Otsuka cluster anneal with soft loop bonds, 2,048 sweeps | 1.44 | No |

### Windows of the glass 0.125 cell

| Cell | Best classical, access ratio (wall ratio) | Energy gap | New-pow wins / ties / losses, solver | New-pow wall, s | Tileable |
| --- | --- | --- | --- | --- | --- |
| `ice-twisted-L9` | CUDA 1.3× (0.4×) | `+1 (1), QPU 1 / tie 6 / CUDA 23, 0.668 s, 30 models` | 7 / 0 / 5 of 12, Otsuka cluster anneal (icefast), 4,096 sweeps | 0.055 | No |
| `ice-twisted-L13` | CPU 28× (1.4×) | `+2 (2), QPU 5 / tie 2 / CPU 23, 116 s, 30 models` | 9 / 0 / 3 of 12, Otsuka cluster anneal (icefast), 16,384 sweeps | 0.691 | No |
| `ice-twisted-L18` | CPU 107× (31×) | `+2 (3), QPU 5 / tie 0 / CPU 25, 119 s, 30 models` | 8 / 1 / 3 of 12, Otsuka cluster anneal (icefast), 16,384 sweeps | 1.25 | Yes |

### Windows of the glass 0.062, 1,200-pair cell

| Cell | Best classical, access ratio (wall ratio) | Energy gap | New-pow wins / ties / losses, solver | New-pow wall, s | Tileable |
| --- | --- | --- | --- | --- | --- |
| `ice-twisted-w62-dimer1200-L9` | CPU 1,171× (343×) | `0 (1), QPU 22 / tie 0 / CPU 8, 131 s, 30 models` | 9 / 0 / 3 of 12, Otsuka cluster anneal (l9_fast) on a 0.096 s deadline | 0.296 | No |
| `ice-twisted-w62-dimer1200-L13` | CUDA 13× (3.9×) | `+2 (1), QPU 7 / tie 1 / CUDA 92, 8.21 s, 100 models` | 12 / 0 / 0 of 12, Otsuka cluster anneal (l9_fast) on a 0.794 s deadline | 0.797 | No |
| `ice-twisted-w62-dimer1200-L18` | CUDA 27× (14×) | `+4 (2), QPU 3 / tie 0 / CUDA 97, 21.4 s, 100 models` | 9 / 0 / 3 of 12, Otsuka cluster anneal, 12,288 sweeps | 4.89 | No |
| `ice-twisted-w62-dimer1200-L22` | CPU 560× (297×) | `-2 (4), QPU 22 / tie 0 / CPU 8, 183 s, 30 models` | 11 / 0 / 1 of 12, Otsuka cluster anneal, 24,576 sweeps | 13.8 | No |

Observations across the family.

- Glass strength without pairs. The new-pow verdict wall grows with glass strength: 0.011 s at 0.031 and 0.045, 0.053 s at 0.062, 0.879 s at 0.090, 11.8 s at 0.125, and 10.9 s at 0.250.
- Pair count at glass 0.062. The verdict wall is 0.053 s for 0 pairs, 1.66 s for 600, 27.0 s for 1,200, 17.0 s for 1,350, 12.5 s for 1,500, and 13.8 s for 1,800. At 1,200 and 1,350 pairs the QPU wins 22 and 19 of 30 models against the 120 s CPU runs. The best classical devices need 390 and 675 times the QPU access.
- Tier 1 cells (no classical beat within 120 s): `ice-twisted-w62-dimer1200`, `ice-twisted-w62-dimer1350`, `ice-twisted-w45-dimer1200`, `ice-twisted-w62-dimer1200-L9`, and `ice-twisted-w62-dimer1200-L22`. The base cell and the window cells of the glass 0.125 cell are tier 4 or 3.
- Pair strength. Half-strength pairs (`p500`) reach a verdict at 6.85 s wall, against 27.0 s for full-strength pairs. The 20-minute CPU rung reaches a median of `-31` units (deepest `-34`, 3 models).
- Window size at glass 0.062 with 1,200 pairs. The verdict wall is 0.296 s for L9 (9 of 12), 0.797 s for L13 (12 of 12), 4.89 s for L18 (9 of 12), 13.8 s for L22 (11 of 12), and 27.0 s for the full chip (23 of 24). The QPU median read offset is 4, 6, 8, 12, and 12 energy units for the same sizes.
- Window size at glass 0.125. The verdict wall is 0.055 s for L9 (7 of 12), 0.691 s for L13 (9 of 12), 1.25 s for L18 (9 of 12), and 11.8 s for the full chip (24 of 24).
- Loops. A 120 s CPU run reaches the target on 30 of 30 models with gap `+10` (5) units. A CUDA cluster anneal at 16,384 sweeps reaches it on 98 of 100 models in 2.53 card-seconds.
- Closing attack. Each cell ends at the new-pow solver named in its Results table. Most cells use an Otsuka cluster anneal. The 1,500-pair cell uses a population anneal with pair and tetrahedron moves. The 1,800-pair cell uses a single-flip anneal with pair and tetrahedron moves.
- Best classical device. CPU is the best device for `w31`, `w45`, `w62`, `w90`, `w62-dimer1200`, `w62-dimer1350`, `w45-dimer1200`, `L13`, `L18`, `w62-dimer1200-L9`, and `w62-dimer1200-L22`. CUDA is the best device for every other cell.

Detail for `ice-twisted-w62-dimer1200` (the 1,200-pair reference cell, tier 1, new-pow verdict Tier 2):

- Beat test. No classical run within 120 s beats the QPU. The best run, CPU `depth-120s-100`, is below the reference on 8 of 30 models. Energy gap `-4 (6)`, QPU 22 / tie 0 / CPU 8, 122 s, 30 models.
- Speed test. CPU `depth-300s-100` reaches the reference at 314 s per model on 30 models, 366 times the QPU wall of 0.86 s. The rank is QPU 0.077 s < CUDA >9.66 s < CPU 314 s. The CUDA gap is >12x (>6.1x), a lower bound from the single-flip GPU ladder at 131,072 sweeps.
- QPU repeat calls. The best energy of a call, highest minus lowest over 11 calls, is 10 units median (8 to 12 units).
- 20-minute depth. The CPU median is `-27` units (3 models), deepest `-30`. QPU deepest-anneal calls median `-4`.
- On seeds, three verdict-kernel CPU runs match 70 of 72 model-runs. Two seeds of the CUDA cluster anneal at 16,384 sweeps match 15 of 24 and 17 of 24 models.
- Chaining. A hash chain of full-chip 1,200-pair links keeps 570 times access (262 times wall) per link and needs one energy check per link. At the first-call target one fresh call qualifies 60 percent of the time (median per model), so a link costs the QPU 0.167 s at 0.1 s per call. One CPU job needs 26.3 s per link. A fleet needs 4.29 s at 100 jobs and 3.76 s at 1,000 jobs.

## Limits and tiling

Limits.

- Each cell uses 12 or 24 nonces for the QPU and new-pow runs, and 30 or 100 common models for the gap tests. The 1,200 s CPU rung covers 1 to 3 models of each cell.
- The repeat-call spread of the QPU is measured for `ice-twisted-w90-dimer1200` only. The best energy of a call has a spread of 9 units (6 to 13), and a fresh call reaches the first-call target with median probability 0.30 per model (pooled 0.45, 54 of 120 calls). It is not measured for the other cells. Longer QPU anneals are measured for the base cell only.
- Small glass strengths (0.031 and 0.045 without pairs) and small windows give classical solvers an easy target.
- The strong layer of the pair-free cell is a b-matching, which is solvable in polynomial time. A SAT solver decides the strong floor of every pair cell in about 0.3 s. The pair count at which the floor stops being reachable is near 1,560.

Per-cell measurements. "Fleet floor" is the link time for one CPU job and for 100 and 1,000 jobs. "20-minute CPU" is the median and deepest margin against the QPU target of the 1,200 s rung, on the number of models that the rung covers. "Read offset" is the median QPU read minus the best read of the same call.

| Cell | Fleet floor, s | 20-minute CPU, energy units | Read offset, energy units |
| --- | --- | --- | --- |
| `ice-twisted` | 5.71 / 2.34 / 2.34 | `-21` median, `-24` deepest, 3 models | 13 |
| `ice-twisted-w31` | 0.012 / 0.012 / 0.012 | `-96`, `-96`, 1 model | 12 |
| `ice-twisted-w45` | 0.016 / 0.016 / 0.016 | `-78`, `-78`, 1 model | 13 |
| `ice-twisted-w62` | 0.088 / 0.08 / 0.08 | `-56`, `-58`, 3 models | 15 |
| `ice-twisted-w90` | 0.877 / 0.877 / 0.877 | `-26`, `-26`, 1 model | 12 |
| `ice-twisted-w250` | 6.49 / 6.49 / 6.49 | `-21`, `-21`, 1 model | 14 |
| `ice-twisted-dimer` | 6.94 / 3.80 / 3.80 | `-25`, `-26`, 3 models | 13 |
| `ice-twisted-dimer1200` | 13.3 / 7.76 / 7.76 | `-22`, `-22`, 1 model | 13 |
| `ice-twisted-w90-dimer` | 7.00 / 2.10 / 2.10 | `-32`, `-33`, 3 models | 13 |
| `ice-twisted-w90-dimer1200` | 16.8 / 8.79 / 8.79 | `-28`, `-31`, 3 models | 13 |
| `ice-twisted-w62-dimer` | 1.89 / 0.916 / 0.916 | `-41`, `-46`, 3 models | 14 |
| `ice-twisted-w62-dimer1200` | 26.3 / 4.29 / 3.76 | `-27`, `-30`, 3 models | 12 |
| `ice-twisted-w62-dimer1350` | 16.7 / 8.80 / 8.80 | `-28`, `-31`, 3 models | 13 |
| `ice-twisted-w62-dimer1500` | 11.5 / 7.53 / 7.53 | `-36`, `-41`, 3 models | 13 |
| `ice-twisted-w62-dimer1800` | 10.8 / 2.86 / 2.86 | `-31`, `-33`, 3 models | 14 |
| `ice-twisted-w45-dimer1200` | 8.33 / 2.28 / 2.28 | `-27`, `-30`, 3 models | 13 |
| `ice-twisted-w62-dimer1200-p500` | 6.24 / 4.58 / 4.58 | `-31`, `-34`, 3 models | 12 |
| `ice-twisted-w62-loop4x300` | 1.40 / 1.08 / 1.08 | `-47`, `-47`, 3 models | 15 |
| `ice-twisted-L9` | 0.12 / 0.09 / 0.09 | `-2`, `-2`, 1 model | 3 |
| `ice-twisted-L13` | 0.747 / 0.689 / 0.689 | `-7`, `-7`, 1 model | 6 |
| `ice-twisted-L18` | 1.50 / 1.26 / 1.26 | `-13`, `-13`, 1 model | 8 |
| `ice-twisted-w62-dimer1200-L9` | 0.107 / 0.096 / 0.096 | `-3`, `-4`, 3 models | 4 |
| `ice-twisted-w62-dimer1200-L13` | 0.332 / 0.096 / 0.096 | `-8`, `-12`, 3 models | 6 |
| `ice-twisted-w62-dimer1200-L18` | 7.66 / 4.83 / 4.83 | `-16`, `-20`, 3 models | 8 |
| `ice-twisted-w62-dimer1200-L22` | 11.6 / 7.43 / 7.43 | `-21`, `-23`, 3 models | 12 |

Tiling. The protocol `tiled-generator-v1` composes 4 hash-chained tiles as a grid with a 30.0 s budget per tile and an equal total direct-solve budget. A cell is tileable if three conditions hold. First, the cut is clean and the generator does not make it. Second, tile and full solutions are within about 100 units and neither always wins. Third, tile targets compose to the global target. All 25 cells pass conditions 1 and 3. A cell fails only condition 2 when one side always wins. The results use 12 nonces per cell. "Delta" is the largest absolute energy delta per tile in units. "Tiled / direct" are the median energies.

| Cell | Tileable | Delta | Tiled wins / ties / direct wins | Tiled / direct median energy | Tile targets reached |
| --- | --- | --- | --- | --- | --- |
| `ice-twisted` | Yes | 8.562500 | 11 / 0 / 1 | `-18,401.5000` / `-18,381.3750` | 37 of 48 |
| `ice-twisted-w31` | No | 1.968500 | 12 / 0 / 0 | `-15,296.5210` / `-15,291.0960` | 48 of 48 |
| `ice-twisted-w45` | Yes | 2.812500 | 11 / 0 / 1 | `-15,763.2900` / `-15,759.1050` | 48 of 48 |
| `ice-twisted-w62` | No | 3.813000 | 12 / 0 / 0 | `-16,333.9360` / `-16,321.5360` | 48 of 48 |
| `ice-twisted-w90` | No | 5.930000 | 12 / 0 / 0 | `-17,261.9500` / `-17,249.1700` | 48 of 48 |
| `ice-twisted-w250` | Yes | 10.125000 | 11 / 0 / 1 | `-22,803.7500` / `-22,781.2500` | 21 of 48 |
| `ice-twisted-dimer` | No | 11.500000 | 12 / 0 / 0 | `-19,983.6250` / `-19,952.3750` | 14 of 48 |
| `ice-twisted-dimer1200` | No | 18.625000 | 12 / 0 / 0 | `-21,537.0000` / `-21,498.8750` | 34 of 48 |
| `ice-twisted-w90-dimer` | No | 8.470000 | 12 / 0 / 0 | `-19,000.5500` / `-18,981.6000` | 22 of 48 |
| `ice-twisted-w90-dimer1200` | No | 12.950000 | 12 / 0 / 0 | `-20,713.9400` / `-20,675.3900` | 17 of 48 |
| `ice-twisted-w62-dimer` | Yes | 5.363000 | 11 / 0 / 1 | `-18,239.2520` / `-18,226.2860` | 48 of 48 |
| `ice-twisted-w62-dimer1200` | Yes | 7.169000 | 6 / 0 / 6 | `-20,133.7640` / `-20,134.5380` | 31 of 48 |
| `ice-twisted-w62-dimer1350` | No | 13.378000 | 12 / 0 / 0 | `-20,556.3140` / `-20,519.8720` | 8 of 48 |
| `ice-twisted-w62-dimer1500` | No | 23.531000 | 12 / 0 / 0 | `-20,979.7920` / `-20,898.7000` | 0 of 48 |
| `ice-twisted-w62-dimer1800` | No | 16.034000 | 12 / 0 / 0 | `-21,894.9620` / `-21,856.2120` | 27 of 48 |
| `ice-twisted-w45-dimer1200` | No | 10.995000 | 12 / 0 / 0 | `-19,781.6100` / `-19,761.6700` | 39 of 48 |
| `ice-twisted-w62-dimer1200-p500` | No | 11.833000 | 12 / 0 / 0 | `-17,795.3940` / `-17,771.0840` | 36 of 48 |
| `ice-twisted-w62-loop4x300` | No | 8.994000 | 12 / 0 / 0 | `-20,309.2780` / `-20,288.2920` | 48 of 48 |
| `ice-twisted-L9` | No | 0.687500 | 12 / 0 / 0 | `-1,850.5000` / `-1,849.6250` | 39 of 48 |
| `ice-twisted-L13` | No | 2.875000 | 12 / 0 / 0 | `-4,302.5000` / `-4,296.6250` | 45 of 48 |
| `ice-twisted-L18` | Yes | 3.812500 | 11 / 0 / 1 | `-9,072.7500` / `-9,058.6250` | 46 of 48 |
| `ice-twisted-w62-dimer1200-L9` | No | 1.829000 | 12 / 0 / 0 | `-2,033.6030` / `-2,028.5860` | 32 of 48 |
| `ice-twisted-w62-dimer1200-L13` | No | 2.608000 | 12 / 0 / 0 | `-4,706.4320` / `-4,698.2720` | 31 of 48 |
| `ice-twisted-w62-dimer1200-L18` | No | 8.018000 | 12 / 0 / 0 | `-9,890.5330` / `-9,872.5540` | 33 of 48 |
| `ice-twisted-w62-dimer1200-L22` | No | 9.077000 | 12 / 0 / 0 | `-15,182.7720` / `-15,160.0880` | 18 of 48 |

Tileable yes: `ice-twisted`, `-w45`, `-w250`, `-w62-dimer`, `-w62-dimer1200`, and `-L18`. The other 19 cells are not tileable.

Chaining. All methods support hash chaining. Pair cells are the best chain base. A hash chain of full-chip `w62-dimer1200` links keeps 570 times access (262 times wall) per link. It removes stashed near misses and does not raise the per-link ratio. Pair-free cells are a poor chain base, because a chain only repeats the per-link ratio. The classical solver matches the QPU in 8.8 to 11.8 s per link at glass 0.125, about 0.1 s at glass 0.062, and 0.018 s at glass 0.031.

Composing windows into a chip-size problem reverses the size scan. Classical solvers match the windows L9 and L13 in 0.30 s and 0.80 s, and the full chip in 26.2 s. Small pieces are weak. A composite built from windows has the same weakness.

## Provenance

Measured in the October 2026 suite discovery run and the new-pow adversary campaign. Research directories under `/home/carback1/quip-data/regimes`:

- `qpu-suite/work/cells/ice-twisted` and `qpu-suite/qpu/ice-twisted` hold the base cell models and captures.
- `qpu-f17/work/cells/<cell_id>` and `qpu-f17/qpu/<cell_id>` hold the models and captures of the other 24 cells.
- `new-pow/<cell_id>`, `new-pow/gpu/measurements/<cell_id>`, `new-pow/tiled-generator/<cell_id>`, `new-pow/qpu-recap/qpu/<cell_id>`, and `new-pow/models-100/<cell_id>` exist for all 25 cells.
- Example for the cell `ice-twisted-w62`: `qpu-f17/work/cells/ice-twisted-w62` and `new-pow/ice-twisted-w62`.
- `new-pow/retime` holds the re-timed verdicts, and `qpu-f17/adversary` holds the adversary results.
