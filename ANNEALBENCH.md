# IsingMark: Annealer Benchmark Tool

Standalone, annealer-agnostic benchmarking tool for Ising-model solvers. IsingMark acts as a coordinator. It spawns a quip miner binary over a Unix socket and feeds it consensus-identical problems, then records the results. It does not join a chain and does not submit work.

Language: Rust (edition 2021, toolchain 1.97.1). License: AGPL-3.0-or-later.

## Why this design

Problem generation, energy scoring, nonce derivation, and session wire types already exist in the `quip-miner` crates (`quip-protocol`, `quip-proto`, `quip-miner-core`). IsingMark reuses those crates. It does not reimplement the draw or the scoring path.

"Add a new annealer" means "ship a quip miner binary that speaks the session protocol." `quip-miner-core` reduces that work to implementing `Sampler::sample` and calling `run`.

IsingMark does not depend on `quip-coordinator`. That crate pulls Substrate and chain stack this tool never needs. Consensus-critical functions live in `quip-protocol` and are used directly.

## Dependencies

Pinned git rev of `https://gitlab.com/quip.network/quip-miner.git` (see `Cargo.toml`):

| Crate | Provides |
|---|---|
| `quip-proto` | Protobuf types (`Job`, `IsingProblem`, `RejectReason`) |
| `quip-protocol` | `draw_ising_milli`, scoring, session handshake, wire codec, derive |
| `quip-miner-core` | `IsingGraph`, sample types used by miners |

IsingMark also uses `clap`, `serde`, `serde_json`, `serde-saphyr` (YAML), `tokio`, and optional `plotters` behind the `plot` feature.

## Module layout

```
src/
  main.rs, cli.rs          clap subcommands; --config YAML; flags override file
  config.rs                YAML load, merge, defaults
  topology.rs              TopologySpec JSON; topology hash
  problem.rs               seed -> blake3(seed||idx) salt -> derive_nonce ->
                           draw_ising_milli
  backend/
    registry.rs            name -> binary path + args; --capabilities/--check
    session.rs             Hello/Welcome/Configure/Job/Result/Reject/Shutdown
  params.rs                hardness [0,1] -> JobParams per BackendKind
  record.rs                JobRecord JSONL; Summary JSON
  stats/{aggregate,ks}.rs  percentiles; KS distance and two-sample p-value
  modes/                   throughput, sweep, comparative, ttt, scaling,
                           reproducibility, compare
  report/{table,plot}.rs   text tables always; plots behind feature plot
```

## Data flow

```
CLI -> load YAML config, merge flags
    -> load TopologySpec (preset fixture or file)
    -> resolve backend from registry
    -> mode driver:
         problem.rs draws jobs from (seed, index)
         params.rs maps hardness to JobParams (where the mode uses hardness)
         session.rs dispatches over UDS, collects Result/Reject
         scoring::energy_milli recomputes energy for each returned spin set
    -> records.jsonl + summary.json
    -> text table; optional PNG if --plot and feature plot
```

Backends are miner binaries invoked with `--quip-coordinator unix://...` and `--miner-id isingmark-0`. Each mode builds a unique socket path under the system temporary directory. Throughput uses `isingmark-<pid>-<n>.sock`, and other modes use a mode-specific prefix. The registry probes each binary with `--capabilities` and `--check` when those paths run.

## Built-in backends

| Name | Binary | Kind |
|---|---|---|
| `cpu-sa` | `quip-cpu-sa` | Sweeping (`num_reads` + `num_sweeps`) |
| `cpu-gibbs` | `quip-cpu-gibbs` | Sweeping |
| `cpu-sb` | `quip-cpu-sb` | Sweeping |
| `cpu-bsb` | `quip-cpu-bsb` | Sweeping |
| `cpu-hdsb` | `quip-cpu-hdsb` | Sweeping |
| `cpu-hbsb` | `quip-cpu-hbsb` | Sweeping |
| `cpu-mps` | `quip-cpu-mps` | Sweeping |
| `cpu-mfa` | `quip-cpu-mfa` | Sweeping |
| `cuda` | `quip-cuda-sa` | Sweeping |
| `cuda-gibbs` | `quip-cuda-gibbs` | Sweeping |
| `metal` | `quip-metal-sa` | Sweeping |
| `dwave` | `quip-dwave-qa` | Annealing (`num_reads` + `anneal_time_us`) |
| `beit` | `quip-beit-sa` | Sweeping |
| `cuopt` | `quip-cuopt-milp` | Annealing |

## Benchmark modes

### throughput

Dispatch jobs for a fixed duration at a fixed hardness. Report jobs per minute and energy stats over completed jobs.

```bash
isingmark throughput --backend cpu-sa --topology-preset smoke --duration 30s
```

### sweep

Run every cell of a parameter grid against the same problem sequence (paired design). Cell parameters ride per-job `IsingProblem` fields, not session-level `SetTarget`. One session per cell.

```bash
isingmark sweep --backend cpu-sa --topology-preset smoke \
  --param-grid '{"num_reads":[64,128],"num_sweeps":[100,500]}' --num-jobs 20
```

`--hardness` is not used: cell parameters come only from the grid. A grid that sets a dimension the backend kind does not use is an error.

### comparative

Binary-search hardness in `[0.0, 1.0]` so this backend's mean best energy crosses a reference JSONL file's mean, then report the final KS distance on the best-energy distributions.

```bash
isingmark comparative --backend cpu-sa --topology-preset smoke \
  --target-file path/to/records.jsonl --num-jobs 20
```

### ttt (time-to-target)

For each energy target (natural units on the CLI, milli-units internally), run trials until the target is reached or `--max-time-s` elapses. Report times for trials that reached the target.

```bash
isingmark ttt --backend cpu-sa --topology-preset smoke \
  --energy-targets "-5,-10,-15" --num-trials 20
```

### scaling

At fixed hardness, generate a random 3-regular topology for each requested node count and measure performance.

```bash
isingmark scaling --backend cpu-sa --sizes 8,16,32 --num-jobs 10
```

### reproducibility

Run two batches against the identical problem sequence and compare best-energy distributions with a two-sample KS test. Reports `differs` against a fixed p-value threshold.

```bash
isingmark reproducibility --backend cpu-sa --topology-preset smoke \
  --num-problems 20
```

### compare

Post-hoc only. It reads two JSONL files and compares their passed best energies, then prints the summary statistics and the KS result. No miner is spawned.

```bash
isingmark compare --file-a results_a.jsonl --file-b results_b.jsonl
```

## Plots

Optional, behind cargo feature `plot` (`plotters`).

| Function | Mode flag | Chart |
|---|---|---|
| `plot_sweep_heatmap` | `sweep --plot PATH` | Mean passed energy over the grid |
| `plot_ttt_cdf` | `ttt --plot PATH` | Empirical CDF of time-to-target |
| `plot_energy_cdfs` | `compare --plot PATH` | Overlayed energy CDFs for two files |

`--plot` always parses. Without the feature, the process exits non-zero and names the rebuild command:

```bash
cargo build --features plot
```

## Result storage

### Per-job records (JSONL)

One `JobRecord` per line: `job_id` (hex), `backend`, `n_solutions`, `best_energy_milli` (optional milli-units; null when no solution), `best_spins_hex` (optional hex of protocol `spins_bytes`, one byte per spin `0x01`/`0xFF`, decodable with `quip_protocol::wire::decode_spins`; null when no solution), `diversity_milli`, `passed`, `rejected`, `reject_reason`, `device_access_time_us`, `wall_ms`, `requested_reads` / `requested_sweeps` / `requested_anneal_time_us` (optional; null when no `SetTarget` was sent, distinct from a genuine zero request), `measured_reads` (optional), `measured_sweeps` (optional). There is no measured anneal time: `SamplerMeta` does not carry one.

Modes that refuse to overwrite refuse a non-empty existing `records.jsonl` before they start.

### Summary (JSON)

`Summary` always includes `mode`, `backend`, `topology` (name, node/edge counts, hash), `hardware`, mode-specific `config`, and aggregate `totals`.

## Topology format

JSON `TopologySpec`: nodes, edges, allowed h/J/spin milli ranges, optional gates. Bundled fixtures live under `fixtures/` (`smoke.spec.json`, `advantage2-system1.spec.json`). Load with `--topology-preset NAME` or `--topology-file PATH`.

## Problem generation

`ProblemSource` draws `count` PoW jobs for a `(seed, topology)` pair. The draw uses `quip_protocol::chacha8::draw_ising_milli` and `derive_nonce` against a zero `last_proof_block_hash` (no chain in benchmark mode). The same seed and topology always yield the same job sequence.

Golden check: `tests/problem_golden.rs` compares job ids against a `quip-coordinator drive` report for seed 42 and the smoke topology.

## Parameter mapping

`params::ParamCurve` maps hardness in `[0.0, 1.0]` to `num_reads` / `num_sweeps` or `anneal_time_us` by linear interpolation. A field value of `0` means "unset" on the wire. Modes that take hardness use a default curve or a YAML `param_curve` block.

## Statistics

- `stats::describe`: min, max, mean, population std (ddof=0), p50/p90/p99
- `stats::ks_distance`: two-sample Kolmogorov-Smirnov statistic
- `stats::ks_two_sample_p`: asymptotic two-sided p-value

Energy comparisons use only `passed == true` records, converted from milli-units to natural units (`/ 1000`).

## CLI and config

Every mode accepts common flags where they apply: `--backend`, `--topology-preset` or `--topology-file`, `--hardness`, `--seed`, `--output-dir`, `--config`.

`--config FILE` loads YAML. Explicit flags override file values. Built-in defaults apply last. See `examples/configs/`.

## Testing policy

- Hermetic tests use `isingmark-fake-miner` (in-tree binary).
- Tests that need real miner binaries fail by default if the binary is missing.
- Set `ISINGMARK_ALLOW_MISSING_MINERS=1` to opt out of that requirement.
- Plot unit tests require `--features plot` and assert a real PNG signature.

## Local quip-miner override

Copy `.cargo/config.toml.example` to `.cargo/config.toml` (gitignored) to patch the three pinned crates to a local checkout. While active, cargo rewrites `Cargo.lock` to path sources. Before commit:

```bash
rg -c 'source = "git\+https://gitlab.com/quip.network/quip-miner' Cargo.lock
# expect 3
```

Delete the config file when you are not co-developing upstream.

## License

AGPL-3.0-or-later. See `LICENSE`.
