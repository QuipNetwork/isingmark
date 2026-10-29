<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Structured datasets

isingmark can replay explicit Ising instances from a file, not only nonce-drawn chain
problems. The first instance files are the structured families of the QPU suite discovery
run. These are Ising problems with a designed structure on the live D-Wave Advantage2 graph.
This page describes the families and their generation, replay, and the measurements that
produced the verdicts.

Every verdict on this page is **provisional**. The suite run was still in progress on
2026-10-15, and the stronger classical attacks were not finished.

## Why these families exist

The chain draws a random Ising problem per nonce. Round 4 of the regime search measured how
the D-Wave Advantage2 QPU, a CUDA single-flip annealer, and CPU cluster kernels compare on
such random problems and on the first structured variants. The report is
`/home/carback1/quip-data/regimes/round4/REPORT.md` on the research host. Its result was that
two-scale structures with a clean magnitude separation fall to classical contraction, so the
suite run searched for structures that keep a QPU advantage.

The suite run screened 19 candidate families in three tiers. The candidate list is
`/home/carback1/quip-data/regimes/qpu-suite/candidates.md`, and the running report is
`/home/carback1/quip-data/regimes/qpu-suite/REPORT.md`. Each family is a deterministic
function of a 32-byte nonce and the live topology specification, so anyone with the generator
and the specification can rebuild the instances bit for bit. isingmark ships the generator and
replays the instances through any registered miner.

## The families

The suite built 15 cells from nine families. Each cell has 24 models, drawn from the nonces
`make_nonces(24, 20260920)`. Every model has 4,575 spins and 41,474 couplers on the live
graph, integer couplings in milli units from −1,000 to 1,000, and zero fields.

| Cell | Family | Tier | Construction | Parameters | Provisional verdict (2026-10-15) |
| --- | --- | --- | --- | --- | --- |
| `ice-glass-r8` | F11 | 1 | Corner-sharing tetrahedra with antiferromagnetic strong bonds at 1,000 milli over a weak random background | `background_milli` 125 | Production QPU (48 reads, 80 µs) wins 23 of 24 against the GPU at 131,072 sweeps and 19 of 24 against CPU loops at 131,072 sweeps (700 s per model). Fails the 20-of-24 CPU gate. The 2,000 µs setting wins 24 of 24. |
| `ice-glass-r5` | F11 | 1 | Same, stronger background | `background_milli` 200 | GPU wins 12 of 12 at 131,072 sweeps. Rejected. |
| `ice-twisted` | F12 | 2 | Tetrahedra with a random gauge per tetrahedron | none | Production QPU wins 21 of 24 against the GPU and 24 of 24 against CPU loops at 131,072 sweeps (925 s per model). Last production candidate. Stronger attacks pending. |
| `tri-match-r4` | F1 | 1 | Three edge-disjoint matchings with random signs over a weak background | `ratio` 4 | GPU wins 12 of 12 at 131,072 sweeps. Rejected. |
| `tri-match-r6` | F1 | 1 | Same, ratio 6 | `ratio` 6 | GPU wins 12 of 12 at 131,072 sweeps. Rejected. |
| `strong-net-d1.5` | F2 | 1 | Random strong-bond network at mean degree 1.5 over a weak background | `density` 1.5 | CPU and GPU reject every setting. |
| `strong-net-d2` | F2 | 1 | Same, mean degree 2 | `density` 2 | CPU and GPU reject every setting. |
| `strong-net-d3` | F2 | 1 | Same, mean degree 3 | `density` 3 | CPU and GPU reject every setting. |
| `diamond-3d` | F15 | 1 | Random ±J on the diamond sublattice | none | GPU wins 11 of 12 at 131,072 sweeps. Rejected. |
| `cubic-no-dimer` | F16 | 1 | Random ±J on the cubic sublattice, no dimer bonds | none | GPU wins 12 of 12 at 131,072 sweeps. Rejected. |
| `dimer-power-mixed` | F6 | 1 | Strong pairs at mixed ratios over a power-law background | none | CPU reduction and polish wins 11 of 12 at 8,192 sweeps. Rejected. |
| `dcl-k44-l5` | F5 | 1 | Deceptive cluster loops on K4,4 cells | `cell_milli` 200 | Solved exactly in 0.19 s per model. Rejected without capture. |
| `dcl-k44-l7` | F5 | 1 | Same, loop ratio 7 | `cell_milli` 143 | Solved exactly in 0.19 s per model. Rejected without capture. |
| `fcl-planted-a0.2` | F4 | 1 | Frustrated cluster loops with a planted ground state | `density_tenths` 2 | The planted state is recoverable from the public nonce in 0.7 ms. Rejected without capture. |
| `fcl-planted-a0.3` | F4 | 1 | Same, density 0.3 | `density_tenths` 3 | Same. Rejected without capture. |

The construction of every cell is in the module docstring of its file under
`tools/structured/isingmark_structured/families/`. The evidence for every verdict is under
`/home/carback1/quip-data/regimes/qpu-suite/work/cells/<cell>/` on the research host, and
the per-cell parameter file in `problems/<family>/params/<cell>.json` records the suite's
generator hash and the census of each model.

## Instance file format

An instance file is JSONL. Line 1 is a header. Every later line is one instance.

```json
{"kind":"header","format":1,"family":"ice-glass-r8","topology_hash":"e66d…a02d","topology_fixture":"chain-h0","nodes":4577,"edges":41515,"generator":"tools/structured","generator_sha256":"d84b…a00c","params":{"background_milli":125},"nonce_rule":"make_nonces(24, 20260920)","suite_topology_sha256":"65ef…47be","created":"2026-10-15T12:00:00Z","verdict":"provisional"}
{"nonce":"e971…615f","h_milli":[0,0,…],"j_milli":[125,-125,1000,…]}
```

| Field | Meaning |
| --- | --- |
| `topology_hash` | The isingmark hash of the topology fixture that the file targets. Replay checks it against `--topology-preset`. |
| `nodes`, `edges` | Counts of the fixture. Replay checks them against the fixture. |
| `nonce` | 64 hex characters. The job id, and the only entropy of the instance. |
| `j_milli` | One integer per fixture edge, in the fixture's edge order. Zero means no coupling. |
| `h_milli` | One integer per fixture node, in the fixture's node order. Optional. Absent means all zero. |
| `reference` | Optional object with reference energies. isingmark ignores it. |

Values are integers from −1,000 to 1,000. The reader rejects a header with another `format`,
a mismatched topology, a wrong `j_milli` or `h_milli` length, and a value out of range.

The suite models live on the live working graph (4,575 qubits, 41,474 couplers). The fixture
`chain-h0` is the same chip with 4,577 qubits and 41,515 couplers. The generator maps every
model to the fixture by qubit id, writes the 41 fixture couplers the suite lacks as zero, and
stops with an error if the fixture lacks a qubit or coupler of the suite specification.

## Generate

The generator is the Python package under `tools/structured/`. It needs Python 3.11 or
later, NumPy, and SciPy. It carries its own ports of the chain's coupling draw and of the
Zephyr coordinate helpers, so it does not need the D-Wave Ocean packages.

```bash
cd tools/structured
python -m venv .venv
.venv/bin/pip install -e '.[test]'
.venv/bin/isingmark-structured --family ice-glass-r8
.venv/bin/isingmark-structured --family ice-twisted --family tri-match-r4 \
  --count 24
```

The generator reads the live topology specification. The default path is
`fixtures/suite-live-2026-09-21.spec.json` in this repository.
Give another copy with `--suite-spec`. Each run writes `<out>/<cell>.jsonl` and
`<out>/<cell>.manifest.json`. Git tracks the manifests. The generator produces the instance
files on demand, and git ignores them, except the two-instance smoke file the tests use.

The port is bit-exact with the suite generators. On 2026-10-15, the verify step matched 24 of
24 models for every one of the 15 cells, and every shared extra array (tetrahedra, gauges,
matchings, clusters) matched too:

```bash
.venv/bin/python -m isingmark_structured.verify \
  --suite-dir /home/carback1/quip-data/regimes/qpu-suite
```

The verify step needs the suite workspace, which exists only on the research host. Exactness
depends on the NumPy bit generator (PCG64 under `default_rng`) and on SciPy's
`maximum_bipartite_matching`. The verify run used NumPy 2.4.6 and SciPy 1.17.1. Run the
verify step again after you upgrade either package. `tools/structured/README.md` has the
full command reference.

## Replay

```bash
isingmark throughput --backend cpu-sa --topology-preset chain-h0 \
  --instances corpora/ice-glass-r8.jsonl --duration 10m
isingmark sweep --backend cpu-sa --topology-preset chain-h0 \
  --instances corpora/ice-twisted.jsonl --sweeps 8192,32768,131072
```

`--instances` and `--corpus` are mutually exclusive. An explicit `--instances` or
`--corpus` flag on the command line displaces the other source's key in the config file.
The header's `topology_hash` must equal the hash of `--topology-preset`. A mismatch fails
before any job runs. Every instance becomes one job with the nonce as its id, the same shape
as a corpus job but with the explicit coefficients attached.

The `throughput` summary records `job_source = "instances:<path>"`, `corpus = null`, and
`corpus_instances = <count>`. The `sweep` summary records `instances = <path>` next to
`corpus`. Every cell of a sweep reloads the file from the start, so all cells measure the
same instances.

## Replicate the suite measurements

The suite compared three arms on the same 24 models per cell. Score every state from the
saved coefficients in integer milli, and compare best energies exactly. Never trust a
solver's own energy report.

### QPU

Native calls on the D-Wave Advantage2 working graph, no minor embedding, production
scaling. Settings and access cost per model:

| Setting | Reads | Anneal | Access per model |
| --- | --- | --- | --- |
| Production | 48 | 80 µs | 0.048 s |
| Ladder | 48 | 200 µs | 0.054 s |
| Ladder | 48 | 500 µs | 0.068 s |
| Ladder | 48 | 2,000 µs | 0.140 s |
| Reference | 256 | 2,000 µs | 0.590 s |

Access time is 33,769 µs + reads × (anneal + 211) µs. The suite recorded 48 reads at 80 µs
on 12 models first, then the ladder on the same 12, then extended the best setting and the
production call to 24 models, and added the 256-read reference for survivors. The capture
worker and request format are in `/home/carback1/quip-data/regimes/qpu-suite/brief.md`.

### GPU

A single-flip weighted simulated-annealing kernel in CUDA on one NVIDIA A4000, 64 reads per
model, at 8,192, 32,768, and 131,072 sweeps. The harness is
`/home/carback1/quip-data/regimes/round4/harness/` and takes JSONL jobs:

```json
{"id": "ice-glass-r8-e971748565fb320b-s131072-r64-msa-i32", "cell": "ice-glass-r8",
 "nonce": "<hex>", "h": [0.0, …], "edges": [[u, v], …], "j": [0.125, -0.125, 1.0, …],
 "beta": [lo, hi], "sweeps": 131072, "reads": 64, "seed": 123, "kernel": "msa", "lane": "i32"}
```

`j` and `h` are milli divided by 1,000. It writes `results.jsonl` with
`kernel_graph_best_milli` and `kernel_s`, the card time per model. At 131,072 sweeps a
4,575-spin model takes about 58 card-seconds.

### CPU

Rust cluster kernels, one core per model, 64 reads, at 8,192, 32,768, and 131,072 sweeps:
exact local blocks, pair (dimer) moves, ice loop moves on the tetrahedron network, and
parallel tempering with isoenergetic cluster moves. These kernels live on the research
host in the `feat/dimer-moves` and `feat/qpu-suite` branches of `quip-miner-cpu`, and the
suite's own kernels sit as uncommitted files in its workspace. They are not published yet.
Until they are, this repository cannot reproduce the CPU arm.

## Limits

- Verdicts are provisional until the suite run ends. The ice families were still under
  attack on 2026-10-15.
- The generator needs the live topology specification, which is not in this repository.
- The CPU kernels are not published.
- The verify step runs only on the research host.
- Bit-exactness depends on the NumPy and SciPy versions named in the Generate section.
