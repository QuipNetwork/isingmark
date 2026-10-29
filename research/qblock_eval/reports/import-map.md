# Research source import map (QUI-1999, EXP-1)

## Purpose

This document records which research sources the repository now holds. It maps each function to a ticket and lists what is still unknown. The research root is
`/home/carback1/quip-data/regimes`. It is read-only and is not a git repository. All
origins below are relative to it.

The repository holds 39 imported files (1783638 bytes) under
`research/qblock_eval/sources/selected/<origin>`. The manifest
`research/qblock_eval/sources/selected/manifest.json` lists each file with its size and blake3
digest. It also lists 218 byte-identical duplicates, 48 retained evidence entries,
and the skipped checks.

Imported kinds: solver, generator, postprocess, build, script, target-manifest, and topology.
No fixture is imported. Frozen fixtures belong to EXP-2 (QUI-2002). The maintainer ruled on 2026-10-29 that no per-file size cap applies.

## How to verify

Run `cargo test --test campaign_source_inventory`. The test reads the manifest. Each listed file must exist and match the recorded size and blake3 digest. It also
checks that every file in the directory is listed, that digests and origins are unique, and
that each duplicate matches its source. A changed byte fails the test.

## Function to ticket map

Each imported function appears once and maps to exactly one ticket. The 93 entries
split by ticket as follows: CPU-1 9, CPU-2 27, CPU-3 15, CPU-7 8, CPU-8 7, CPU-9 6, EXP-2 1, IM-2 2, IM-3 2, SC-8 3, SC-9 7, SC-10 5, SC-11 1. Files of kind build, script, target-manifest, and topology
have no functions.

Ticket reasoning:

- CPU-1 reconciles the shared SA core. The `sa_msc`, `sa_sampler`, `sampler_core`, and `coloring` entries, and the `clusters` and `unit_encoding` helpers, belong to it.
- CPU-2 ports the weighted dimer sampler. `validate_common`, `MAX_READS`, `MAX_BETA`, `check_beta`, and `energy_f64` go here. The first four bound the inputs of the sampler. `energy_f64` is the parity oracle in the tests.
- CPU-3 ports the Otsuka cluster update and its graph preparation. CPU-4 exposes it as a binary, so no function maps to CPU-4.
- `Exchange::step` maps to CPU-3. It is also a reference for CU-2. The `pair_bench` kernels are also a reference for CU-1.
- `VARIANTS` and `set_pair_strength` map to SC-9. `VARIANTS` holds the w62-dimer1200 recipe and `set_pair_strength` sets the pair bond strength of the p500 recipe.
- SC-8 to SC-11 are the generator tickets. IM-2 and IM-3 are the archive loader and parity gate. CPU-7 to CPU-9 are the postprocess ports. `validate_baseline` maps to EXP-2 because it checks frozen fixtures.

| Function | Source id | Ticket |
|---|---|---|
| `sample` | `cpu-dimer-weighted` | CPU-2 |
| `anneal` | `cpu-dimer-weighted` | CPU-2 |
| `threshold` | `cpu-dimer-weighted` | CPU-2 |
| `Group::build` | `cpu-dimer-weighted` | CPU-2 |
| `Group::minimize` | `cpu-dimer-weighted` | CPU-2 |
| `State::apply` | `cpu-dimer-weighted` | CPU-2 |
| `State::single` | `cpu-dimer-weighted` | CPU-2 |
| `IceTopology::new` | `cpu-dimer-weighted-ice` | CPU-2 |
| `IceWorkspace::step` | `cpu-dimer-weighted-ice` | CPU-2 |
| `IceWorkspace::defect_step` | `cpu-dimer-weighted-ice` | CPU-2 |
| `IceWorkspace::propose` | `cpu-dimer-weighted-ice` | CPU-2 |
| `Paths::new` | `cpu-dimer-weighted-paths` | CPU-2 |
| `Paths::minimize` | `cpu-dimer-weighted-paths` | CPU-2 |
| `sample` | `cpu-dimer-weighted-tempering` | CPU-2 |
| `sample_band` | `cpu-dimer-weighted-tempering` | CPU-2 |
| `replica_move` | `cpu-dimer-weighted-tempering` | CPU-2 |
| `exchange` | `cpu-dimer-weighted-tempering` | CPU-2 |
| `adapt_ladder` | `cpu-dimer-weighted-tempering` | CPU-2 |
| `contract` | `cpu-dimer-research` | CPU-2 |
| `sample_dimer` | `cpu-dimer-research` | CPU-2 |
| `sample_clusters` | `cpu-dimer-research` | CPU-2 |
| `DimerVariant` | `cpu-dimer-research` | CPU-2 |
| `clusters` | `cpu-dimer-research` | CPU-1 |
| `unit_encoding` | `cpu-dimer-research` | CPU-1 |
| `validate_common` | `cpu-research` | CPU-2 |
| `MAX_READS` | `cpu-sa-msc-f64` | CPU-2 |
| `MAX_BETA` | `cpu-sa-msc-f64` | CPU-2 |
| `check_beta` | `cpu-sa-msc-f64` | CPU-2 |
| `energy_f64` | `cpu-sa-msc-f64` | CPU-2 |
| `MscState` | `cpu-sa-msc` | CPU-1 |
| `LANES` | `cpu-sa-msc` | CPU-1 |
| `read_rng` | `cpu-sa-sampler` | CPU-1 |
| `CpuGraph` | `cpu-sampler-core` | CPU-1 |
| `build_beta_schedule` | `cpu-sampler-core` | CPU-1 |
| `build_seeded_beta_schedule` | `cpu-sampler-core` | CPU-1 |
| `Coloring` | `cpu-coloring` | CPU-1 |
| `Graph::new` | `cpu-pair-bench` | CPU-3 |
| `Workspace::partition` | `cpu-pair-bench` | CPU-3 |
| `Workspace::sweep` | `cpu-pair-bench` | CPU-3 |
| `unit_sweep` | `cpu-pair-bench` | CPU-3 |
| `single_sweep` | `cpu-pair-bench` | CPU-3 |
| `houdayer` | `cpu-pair-bench` | CPU-3 |
| `StripTopology::new` | `cpu-strips` | CPU-3 |
| `StripTopology::sweep` | `cpu-strips` | CPU-3 |
| `Graph::new` | `cpu-fast-bench` | CPU-3 |
| `Work::partition` | `cpu-fast-bench` | CPU-3 |
| `Work::sweep` | `cpu-fast-bench` | CPU-3 |
| `single_sweep` | `cpu-fast-bench` | CPU-3 |
| `anneal_group` | `cpu-fast-bench` | CPU-3 |
| `solve` | `cpu-fast-bench` | CPU-3 |
| `Exchange::step` | `cpu-fast-bench` | CPU-3 |
| `VARIANTS` | `f17-generate2` | SC-9 |
| `add_dimers` | `f17-generate2` | SC-9 |
| `rescale_weak` | `f17-generate2` | SC-9 |
| `rng_for` | `f17-generate2` | SC-9 |
| `build` | `f17-wave-run` | SC-9 |
| `set_pair_strength` | `f17-wave-run` | SC-9 |
| `wave` | `newpow-gen100` | SC-9 |
| `k4_cell` | `newpow-gen100` | SC-10 |
| `twisted_base` | `newpow-gen100` | IM-2 |
| `nonce_for` | `newpow-gen100` | IM-2 |
| `check` | `newpow-gen100` | IM-3 |
| `compare` | `newpow-gen100` | IM-3 |
| `build` | `f17-generate3` | SC-10 |
| `main` | `f17-generate3` | SC-10 |
| `load_graph` | `f17-generate3` | SC-10 |
| `rng_for` | `f17-generate3` | SC-10 |
| `topology` | `suite-ice` | SC-8 |
| `generate_model` | `suite-ice` | SC-8 |
| `generate_model` | `suite-ice-twisted` | SC-8 |
| `generate_model` | `suite-ice-fields` | SC-11 |
| `validate_baseline` | `suite-ice-fields` | EXP-2 |
| `Ising` | `pp-descent` | CPU-7 |
| `natural_moves` | `pp-descent` | CPU-7 |
| `MoveSet` | `pp-descent` | CPU-7 |
| `GreedyC` | `pp-descent` | CPU-7 |
| `ice_tiles` | `pp-descent` | CPU-7 |
| `greedy_descent` | `pp-greedy-c` | CPU-7 |
| `extract_groups` | `pp-prep-groups` | CPU-7 |
| `PairWorm` | `pp-pairs-py` | CPU-8 |
| `pair_worm_descent` | `pp-pairworm-c` | CPU-8 |
| `crossover` | `pp-pairworm-c` | CPU-8 |
| `defect_counts` | `pp-pairworm-c` | CPU-8 |
| `schedule_pw` | `pp-pairs-run` | CPU-8 |
| `alternate` | `pp-pairs-run` | CPU-8 |
| `Context` | `pp-pairs-run` | CPU-8 |
| `Budget` | `pp-pairs-run` | CPU-7 |
| `TileTables` | `pp-icetiles-py` | CPU-9 |
| `WormC` | `pp-icetiles-py` | CPU-9 |
| `worm_run` | `pp-icetiles-c` | CPU-9 |
| `parse` | `pp-icetiles-method` | CPU-9 |
| `Prepared` | `pp-icetiles-method` | CPU-9 |
| `run` | `pp-icetiles-method` | CPU-9 |

## CPU fork comparison

The research root has 15 CPU source trees and the `new-pow/diversity/solvers/fast_bench` crate
with the same layout. Four other directories named `cpu` hold per-nonce output and no source.

The representative tree is `new-pow/ice-twisted-w62-dimer1200/cpu`. It is the preferred
lineage. It is not a strict superset, because some `examples/` files exist only in other trees.

Every `Cargo.toml`, `src/`, `src/bin/`, and `tests/` file is byte-identical across all trees.
The only differences are in `examples/`. Five paths differ: `pair_bench.rs` (six variants),
`fast_bench.rs`, `l9_fast.rs`, `pair_par.rs`, and `sb_bench.rs`.

Algorithm deltas, none imported:

- D1: `w62-loop4x300` `pair_bench.rs` uses union-find units for closed 4-loops. It serves CPU-3 evidence and CPU-12.
- D2: `L18`, `L22`, and `p500` `pair_bench.rs` add `Graph::pair_strength` for pair bonds other than `-1000` milli. It also holds the knee schedule.
- D3: `pair_par.rs` runs threaded replica anneals with synchronization segments. It changes execution, not the single-replica kernel.
- D4: `l9_fast.rs` is a lean multi-thread Otsuka anneal with a wall-deadline mode. The two variants differ only in state output.

Parameter deltas: P1 adds `--beta-knee` and `--knee-frac` (L9 and L13). P2 adds `--beta-knot` and `--knot-fraction` on the older non-CSR base. Both are off by default.

Non-algorithm deltas: N1 is the older `pair_bench.rs` base. It has no CSR layout and no incremental boundary sums.
The single-replica move set is the same. This was read, not run. N2 is output-only variants of `fast_bench.rs`. N3 is `sb_bench.rs`, which is unrelated.

`fast_bench.rs` is a separate solver in the same family as `pair_bench` in soft pair mode. It adds a CSR layout, an
incremental heat bath, replica exchange, and an exact strip polish. The strip polish lives in `examples/cluster/strips.rs`, which is
imported as `cpu-strips` and is identical in every tree.

## Dimer recipes and extended cohorts

The three recipes are `ice-twisted-w62-dimer1200`, `ice-twisted-w62-dimer1350`, and
`ice-twisted-w45-dimer1200`. All use the same steps:

1. Take the `ice-twisted` base model of the nonce (`ice_twisted.py`, `generate_model`).
2. Run `add_dimers`: permute the edges with |J| = `125` and place pairs at J = `-1000` until the pair count is reached. A pair is skipped if either endpoint is used.
3. Run `rescale_weak`: set every |J| = `125` edge to the weak value.

`VARIANTS` in `generate2.py` (lines 149 to 165) holds only the first recipe, built as `rescale_weak(add_dimers(m, n, 1200), 62)`. The p500 recipe differs in one step: `wave` calls `set_pair_strength` when `pair_milli` is not `-1000`, which sets every pair bond to `-500`. Step 2 must run before step 3. After the rescale no |J| = `125` edge is left, so no pair can be placed.

| Recipe | Weak (milli) | Pairs | Set by |
|---|---|---|---|
| w62-dimer1200 | 62 | 1200 | `generate2.py` `VARIANTS`, `gen100.py` `wave` |
| w62-dimer1350 | 62 | 1350 | `wave_run.py --pairs 1350 --weak 62` |
| w45-dimer1200 | 45 | 1200 | `wave_run.py --pairs 1200 --weak 45` |
| w62-dimer1200-p500 | 62 | 1200 | `wave_run.py build --pairs 1200 --weak 62 --pair-milli -500`, `gen100.py` `wave` |

The code does not check that pairs avoid tetrahedron bonds, although the `generate2.py` docstring says so.
Pairs can share spins with tiles. SC-9 must not copy the docstring claim as a rule.

Later parameter recipes: charged ice (`generate3.py`, scale `250`, background `+-31`, charge q drawn from
`{0, +2, -2}` with p = `0.5, 0.25, 0.25`) and field ice h500 and h1000 (`ice_fields.py`, 285 field spins).

Extended cohorts. The master list has 24 nonces (`qpu-suite/work/cells/ice-twisted/nonces.json`, made by
`default_rng(20260920)`). The screen cohort is its first 12. Nonces from index 13 on are
`sha256(b"new-pow/models-100/" + str(i))` (`gen100.py`, `nonce_for`). Each cell reaches 100
nonces. The `record.json` file of each cell in `new-pow/models-100/` holds the `index_map` and the reproduction check. The README cohort table is
incomplete, so use the `record.json` files. These fixtures are cited here and are not imported.

| Cell | Saved models | Extension indices | Total |
|---|---|---|---|
| w62-dimer1200 | 24 | 13 to 88 | 100 |
| w62-dimer1350 | 24 | 13 to 88 | 100 |
| w45-dimer1200 | 12 | 13 to 100 | 100 |
| ice-charge | 12 | 13 to 100 | 100 |
| ice-fields-h500 | 24 | 13 to 88 | 100 |
| ice-fields-h1000 | 12 | 13 to 100 | 100 |

## Provenance

RNG. The CPU kernels use `rand` 0.8 `SmallRng`, seeded per read by `read_rng`. The dimer sampler
salts, copied from `dimer_weighted.rs` lines 22 to 24, are `SINGLE_SALT` `0x5745_4947_4854_3031`, `GROUP_SALT` `0x4752_4f55_5053_3031`, and `ICE_SALT`
`0x4943_454c_4f4f_5031`. `threshold` draws u from `Open01` and returns `floor(-ln(u) * SCALE / (2 beta))`, capped at `i32::MAX`, as an `i32` (`SCALE` = 1000).
The Python generators use NumPy `default_rng` (PCG64). Tile order and gauges use
`default_rng(int.from_bytes(sha256(bytes.fromhex(nonce)).digest(), "big"))` (`ice.py`). Dimer choice
and charge draws use `rng_for(nonce, tag)`, which is
`default_rng(int.from_bytes(sha256(tag.encode() + bytes.fromhex(nonce)).digest()[:8], "little"))`.
The tag is `"dimer"` for `add_dimers` and the cell name for `generate3.py`. Weak-layer signs come from the production stream
`quip_msa.draw_ising` (package `quip-msa` 0.1.0, source not read). SC-9 historical parity needs a pinned build.
Field ice draws its magnitudes through `regimes.draw_couplings` with the nonce `sha256(bytes.fromhex(nonce) + FIELD_DOMAIN).hexdigest()`, where `FIELD_DOMAIN` is `b"ice-fields-v1"` (`ice_fields.py` line 44).
The float kernel `sa_msc_f64.rs` takes initial words from `read_rng` and thresholds from `SmallRng` seeded with `params.seed ^ THRESHOLD_SALT` (`0x4D53_415F_4636_3401`, scheme `shared-node-threshold-v1`). The strip polish in `strips.rs` uses the caller `SmallRng`, which `fast_bench.rs` seeds with `base_seed ^ 0x9e37_79b9_7f4a_7c15`.

Matching library. Tile matching uses `scipy.sparse.csgraph.maximum_bipartite_matching` (numpy 2.4.6,
scipy 1.17.1, networkx 3.6.1, recorded in the `ice-twisted` cell manifest). Dimer matching uses no library. It is a greedy
walk over a permutation. No CPU tree uses a matching crate.

Input limits. `validate_common` rejects graphs with more than `DEFAULT_MAX_NODES` = `100000` nodes or `DEFAULT_MAX_EDGES` = `1000000` edges. Both constants are defined in `src/lib.rs` (lines 68 and 69), which is not imported. `sa_msc_f64.rs` supplies `MAX_READS` = `4096` and `MAX_BETA` = `1e100`.

Topology. `topology-live-2026-09-21.spec.json` (589231 bytes) holds 4575 nodes and 41474 edges. Every generator loads it. It is imported.

Postprocess. The three dimer cells use the `ice-pairs` family with method `pw` and 0.76 s of CPU per QPU call. The `ice-charge` and field-ice cells use `ice-tiles` with 0.25 s.
The reference is `qpu80_pp = min(pp_best, qpu80_greedy)`. `targets-pp.json` holds 12 rows per cell. The 100-row file is not imported.
`descent.py` also needs `dimod`, `dwave.samplers`, `scipy.sparse`, `rescore.py`, and `prep_groups.py`. Prebuilt `.so` files are not imported.

License. No `LICENSE` file exists in any source tree. The `Cargo.toml` of the CPU trees declares
`license = "AGPL-3.0-or-later"`. That field is the evidence for the Rust files. The Python, C,
Makefile, and JSON files have no license evidence.

### Redistribution granted

Evidence: `new-pow/ice-twisted-w62-dimer1200/cpu/Cargo.toml` (license field).

| Source id | Origin |
|---|---|
| `cpu-cargo-toml` | `new-pow/ice-twisted-w62-dimer1200/cpu/Cargo.toml` |
| `cpu-dimer-weighted` | `new-pow/ice-twisted-w62-dimer1200/cpu/src/dimer_weighted.rs` |
| `cpu-dimer-weighted-ice` | `new-pow/ice-twisted-w62-dimer1200/cpu/src/dimer_weighted/ice.rs` |
| `cpu-dimer-weighted-paths` | `new-pow/ice-twisted-w62-dimer1200/cpu/src/dimer_weighted/paths.rs` |
| `cpu-dimer-weighted-tempering` | `new-pow/ice-twisted-w62-dimer1200/cpu/src/dimer_weighted/tempering.rs` |
| `cpu-dimer-research` | `new-pow/ice-twisted-w62-dimer1200/cpu/src/dimer_research.rs` |
| `cpu-research` | `new-pow/ice-twisted-w62-dimer1200/cpu/src/research.rs` |
| `cpu-sa-msc-f64` | `new-pow/ice-twisted-w62-dimer1200/cpu/src/sa_msc_f64.rs` |
| `cpu-sa-msc` | `new-pow/ice-twisted-w62-dimer1200/cpu/src/sa_msc.rs` |
| `cpu-sa-sampler` | `new-pow/ice-twisted-w62-dimer1200/cpu/src/sa_sampler.rs` |
| `cpu-sampler-core` | `new-pow/ice-twisted-w62-dimer1200/cpu/src/sampler_core.rs` |
| `cpu-coloring` | `new-pow/ice-twisted-w62-dimer1200/cpu/src/coloring.rs` |
| `cpu-pair-bench` | `new-pow/ice-twisted-w62-dimer1200/cpu/examples/pair_bench.rs` |
| `cpu-strips` | `new-pow/diversity/solvers/fast_bench/examples/cluster/strips.rs` |
| `cpu-fast-bench` | `new-pow/diversity/solvers/fast_bench/examples/fast_bench.rs` |

### Redistribution unrecorded (missing rights)

Each of these 24 files has no license evidence. The maintainer directed the copy. The owner must record a grant or
remove the file before any external release.

| Source id | Origin |
|---|---|
| `f17-generate2` | `qpu-f17/generate2.py` |
| `f17-wave-run` | `qpu-f17/wave_run.py` |
| `newpow-gen100` | `new-pow/models-100/gen100.py` |
| `f17-generate3` | `qpu-f17/generate3.py` |
| `suite-ice` | `qpu-suite/work/ice.py` |
| `suite-ice-twisted` | `qpu-suite/work/ice_twisted.py` |
| `suite-ice-fields` | `qpu-suite/work/ice_fields.py` |
| `pp-descent` | `new-pow/postprocess/descent.py` |
| `pp-greedy-c` | `new-pow/postprocess/greedy.c` |
| `pp-prep-groups` | `new-pow/gpu/scripts/prep_groups.py` |
| `pp-pairs-py` | `new-pow/postprocess/optimizers/ice-pairs/pairs.py` |
| `pp-pairworm-c` | `new-pow/postprocess/optimizers/ice-pairs/pairworm.c` |
| `pp-pairs-run` | `new-pow/postprocess/optimizers/ice-pairs/run.py` |
| `pp-icetiles-py` | `new-pow/postprocess/optimizers/ice-tiles/icetiles.py` |
| `pp-icetiles-c` | `new-pow/postprocess/optimizers/ice-tiles/icetiles.c` |
| `pp-icetiles-method` | `new-pow/postprocess/optimizers/ice-tiles/method.py` |
| `pp-pairs-makefile` | `new-pow/postprocess/optimizers/ice-pairs/Makefile` |
| `pp-tiles-makefile` | `new-pow/postprocess/optimizers/ice-tiles/Makefile` |
| `newpow-rescore` | `new-pow/rescore.py` |
| `pp-write-targets-100` | `new-pow/postprocess/write_targets_pp_100.py` |
| `pp-icetiles-specs` | `new-pow/postprocess/optimizers/ice-tiles/specs.json` |
| `targets-pp` | `new-pow/targets-pp.json` |
| `f17-targets` | `qpu-f17/adversary/targets.json` |
| `topology-live` | `topology-live-2026-09-21.spec.json` |

## Retained research evidence

These items stay in the research root. The manifest lists each under `retained_evidence`.

- Frozen fixtures, imported by EXP-2 (QUI-2002): nonce lists, cell manifests, `record.json` files, `.npz` models, and `f24-plan.json` with `f24-runtime.json`.
- Measurement-scale data: `new-pow/targets-pp-100.json` and `new-pow/models-100/manifest.json`.
- CPU fork variants D1 to D4, P1, P2, and N1 to N3, the `src/bin`, `tests`, and other kernels, and the sibling solver directories.
- QPU capture machinery, classical attack code, and optimizer reports.

The manifest lists 218 duplicate paths. They point at the imported sources. The `Cargo.toml` and the dimer files appear in all 15 trees.

## Skipped checks

- SC-2 golden format: not defined yet (QUI-2000, Backlog).
- SC-7 conformance checks: not defined yet (QUI-2019, Backlog).
- sha256sum check replaced by blake3 per maintainer.

## Open items

- `src/lib.rs` is not imported. Only its two limit constants are recorded above.
- The ticket title list (`tickets.json`) was unreadable, so ticket assignments follow the work plan and the umbrella table. Change them in the manifest if the table differs.
