# IsingMark Rust Design

Supersedes `2026-04-16-isingmark-design.md`. The Python code is replaced, not
extended.

## Purpose

Benchmark quip Ising miners by acting as a coordinator. IsingMark spawns miner
binaries over a Unix socket, feeds them consensus-identical problems, and
measures what comes back. It never joins a chain and never submits work.

"Add a new annealer" means "write a quip miner binary." `quip-miner-core`
reduces that to writing `Sampler::sample` and calling `run`.

## Why the rewrite

The Python tree reimplements infrastructure that already exists in Rust in the
`quip-miner` repo, and it does so at lower fidelity. `quip-coordinator` has a
drive mode covering problem generation, validation, aggregation, and JSONL
output. Its `RandomSource` draws through `draw_ising_milli`, the same code the
network uses, so its problems match production by construction. The Python port
reproduces that logic separately and can drift from it.

Three gaps in the Python tree drove the decision:

- `ProblemFeeder`, `sa_params`, `qpu_params`, `plot_sweep_heatmap`,
  `plot_ttt_cdf`, and `format_summary_table` are implemented and tested, but no
  CLI path imports them.
- The documented `mp.Process` worker pool and `--workers` flag do not exist.
- `run_throughput` reads `params["num_sweeps"]` unconditionally, so
  `QPUParamsAdaptor`, which returns `annealing_time`, raises `KeyError`.

## Dependencies

Source repo: `https://gitlab.com/quip.network/quip-miner.git`, branch `v0.3`.
The repo is public, so builds and CI need no credentials. Pin a rev rather than
a branch and bump it deliberately; `623af0700d36359687b1e69d866ae782d932a0e9`
is the current v0.3 head.

| Crate | Provides |
|---|---|
| `quip-proto` | Protobuf types (`Job`, `IsingProblem`, `RejectReason`) |
| `quip-protocol` | `chacha8::draw_ising_milli`, `scoring`, `session`, `wire`, `derive` |
| `quip-miner-core` | `IsingGraph`, `SampleParams`, `SamplerResult`, `Algorithm` |

Those three pull only `tonic`, `prost`, `tokio`, and `blake3`. IsingMark adds
`clap`, `serde`, `serde_json`, and `serde_yaml`. The `plot` feature adds
`plotters`.

IsingMark does not depend on `quip-coordinator`. That crate hard-deps
`sp-core` from the QuipNetwork polkadot-sdk fork, `subxt`, `jsonrpsee`, `axum`,
and two crates from `quip-validator`. Reusing its drive harness would save
about 500 lines of gRPC plumbing and cost the entire Substrate stack, for a tool
that never touches a chain. It would save no fidelity: every consensus-critical
function lives in `quip-protocol`.

Toolchain pinned to 1.97.1, matching the miners. The lint profile is copied from
`quip-miner-cpu`: `panic`, `unwrap-used`, and `todo` deny; `pedantic` warns.
`.cargo/config.toml` carries a `[patch]` block pointing at a local `quip-miner`
checkout for iteration.

## Module layout

```
src/
  main.rs, cli.rs          clap subcommands; --config YAML, flags override file
  topology.rs              TopologySpec JSON: nodes, edges, allowed_h_milli,
                           allowed_j_milli, allowed_spin_milli, gates
  problem.rs               seed -> blake3(seed||idx) salt -> derive_nonce ->
                           draw_ising_milli
  backend/
    registry.rs            name -> binary path + args; --capabilities/--check probe
    session.rs             coordinator side: Hello/Welcome/Configure/Job/
                           Result/Reject/Shutdown, credit flow
  params.rs                hardness [0,1] -> SampleParams, per-backend curves
  record.rs                per-job record, JSONL writer
  stats/{aggregate,ks}.rs  percentiles; KS distance and two-sample KS test
  modes/                   throughput, sweep, comparative, ttt, scaling,
                           reproducibility, compare
  report/{table,plot}.rs   text tables always; plots behind the `plot` feature
```

## Reused versus written

Reused, never reimplemented:

| Function | Crate | Role |
|---|---|---|
| `chacha8::draw_ising_milli` | `quip-protocol` | Problem draw |
| `derive::derive_nonce` | `quip-protocol` | Nonce derivation |
| `scoring::energy_milli` | `quip-protocol` | Energy |
| `scoring::set_diversity` | `quip-protocol` | Diversity |
| `session::build_hello`, `check_welcome` | `quip-protocol` | Handshake |
| `wire::{encode_i32_le, decode_spins, ...}` | `quip-protocol` | Codec |

Written here: the coordinator-side session loop, benchmark modes B through F,
`compare`, the KS and percentile statistics, multi-backend orchestration,
plots, and YAML config loading.

## Data flow

```
CLI -> load YAML config, merge flags
    -> load TopologySpec (fixture or file)
    -> per backend: spawn binary on UDS, handshake, Configure
    -> mode driver:
         problem.rs draws jobs from (seed, index)
         params.rs resolves hardness -> SampleParams
         session.rs dispatches, collects Result/Reject
         scoring::energy_milli and set_diversity validate
    -> JSONL rows plus summary JSON (topology, hardware, config blocks)
    -> text table; optional plots
```

Backends are miner binaries invoked with
`--quip-coordinator unix:///tmp/isingmark-<pid>.sock`. The registry probes each
with `--capabilities` for its JSON capability block and `--check` to confirm the
backend runs before a benchmark starts.

## Benchmark modes

| Mode | Measures |
|---|---|
| `throughput` | Jobs per minute over a fixed duration |
| `sweep` | Grid over `num_reads` x `num_sweeps` or `anneal_time_us` |
| `comparative` | Binary search on hardness for the smallest KS distance between backend B's energy CDF and A's |
| `ttt` | Time-to-target CDF; median and p99 |
| `scaling` | Performance against node count |
| `reproducibility` | Two-sample KS across repeated identical runs |
| `compare` | Two result files side by side |

## Testing

- Golden: the problem draw checked against
  `quip-miner/conformance/golden_vectors.json`, and cross-checked against
  `quip-coordinator drive --source random` for the same `(seed, topology)`.
- Session conformance: drive the real `quip-cpu-sa` binary against
  `smoke.spec.json` (8 nodes, 8 edges), fast enough for CI.
- Statistics: KS distance and two-sample KS against hand-computed values;
  percentile edge cases including empty and single-element inputs.
- Mode integration: each mode over the smoke topology, asserting JSONL shape
  and summary fields.
- Recovery: generate results at hardness 0.3, confirm `comparative` recovers
  approximately 0.3.

## Migration

Delete `isingmark/`, `tests/`, `pyproject.toml`, and `examples/` in the same
branch. Rewrite `ANNEALBENCH.md` for the Rust design. Keep `LICENSE`
(AGPL-3.0-or-later). Copy `advantage2-system1.spec.json` (4577 nodes, 41515
edges) and `smoke.spec.json` from `quip-coordinator/fixtures/drive/` into
`fixtures/`.

## Risks

- **Branch churn.** v0.3 is unreleased. Pinning a rev contains this; bumping it
  is a deliberate change with a test run.
- **No git remote.** This repo has none configured. CI cannot run until one
  exists.
- **Backend coverage.** CI on this Darwin host covers CPU backends only. Metal
  is testable locally, CUDA is not, and the D-Wave backend needs Leap
  credentials. Those backends get registry entries and probe support without CI
  coverage.
- **Session drift.** Writing our own coordinator session means it can diverge
  from production behavior. The conformance test against a real miner binary is
  the guard; extend it whenever the miner protocol changes.
