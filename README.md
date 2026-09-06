# IsingMark

Annealer-agnostic Ising benchmark CLI. It spawns a quip miner binary and feeds it consensus-identical problems. Each run writes JSONL records plus a summary JSON. It does not join a chain.

## Install and build

Requires Rust 1.97.1 (`rust-toolchain.toml`) and `protoc`, the protobuf
compiler. The `quip-proto` dependency generates its gRPC bindings at build
time with prost-build, which shells out to `protoc`. Without it the build
fails at `quip-proto` with `Could not find `protoc``.

```bash
brew install protobuf              # macOS
apt-get install protobuf-compiler  # Debian and Ubuntu
```

```bash
cargo build --release
cargo install --path .
isingmark --help
```

Optional PNG plots:

```bash
cargo build --release --features plot
```

## Subcommands

Examples assume `quip-cpu-sa` (or another registered miner) is on `$PATH`.

### throughput

Jobs per minute at fixed hardness for a fixed duration.

```bash
isingmark throughput --backend cpu-sa --topology-preset smoke --duration 30s
```

Replay problems the chain actually posed, instead of deriving them from a
seed:

```bash
isingmark throughput --backend cpu-sa \
  --topology-preset chain-h0 --corpus chain-h0 --duration 10m
```

Two chain corpora ship with the crate: `chain-h0` (4658 instances) and
`chain-ternary` (3148). Each must be paired with the topology preset of the
same name, and isingmark refuses a mismatched pair rather than replaying the
wrong problems. See `docs/datasets.md`.

### sweep

Parameter grid over the same problem set (paired design).

```bash
isingmark sweep --backend cpu-sa --topology-preset smoke \
  --param-grid '{"num_reads":[64,128],"num_sweeps":[100,500]}' --num-jobs 5
```

With plots (binary built with `--features plot`):

```bash
isingmark sweep --backend cpu-sa --topology-preset smoke \
  --param-grid '{"num_reads":[64,128],"num_sweeps":[100,500]}' \
  --plot out/sweep.png
```

Sweep a corpus instead of seed-derived problems. Every cell replays the same
instances, so a difference between cells comes from the parameters and not
from the problems:

```bash
isingmark sweep --backend dwave \
  --topology-preset chain-h0 --corpus chain-h0 \
  --param-grid '{"num_reads":[16,32],"anneal_time_us":[80,120]}' --num-jobs 250
```

A corpus is finite, so `--num-jobs` becomes an upper bound: each cell draws
the first `min(--num-jobs, corpus length)` instances.

### ttt

Time-to-target for each energy level (natural units on the CLI).

```bash
isingmark ttt --backend cpu-sa --topology-preset smoke \
  --energy-targets "-5,-10" --num-trials 5 --plot out/ttt.png
```

### comparative

Search hardness so this backend matches a reference energy distribution.

```bash
isingmark comparative --backend cpu-sa --topology-preset smoke \
  --target-file path/to/records.jsonl
```

### scaling

Fixed hardness across node counts (random 3-regular topologies).

```bash
isingmark scaling --backend cpu-sa --sizes 8,16 --num-jobs 5
```

### reproducibility

Two batches on the same problem sequence, compared with a two-sample KS test.

```bash
isingmark reproducibility --backend cpu-sa --topology-preset smoke \
  --num-problems 10
```

### compare

Post-hoc comparison of two JSONL result files. No miner is spawned.

```bash
isingmark compare --file-a results_a.jsonl --file-b results_b.jsonl
isingmark compare --file-a a.jsonl --file-b b.jsonl --plot out/cdfs.png
```

## Configuration files

A YAML file can supply backend, topology, and every other option a mode accepts, so a single `--config` flag is enough:

```bash
isingmark throughput --config examples/configs/throughput_cpu.yaml
isingmark sweep --config examples/configs/sweep_cpu.yaml
```

Explicit flags override the file. Built-in defaults apply when both omit a field. See `examples/configs/` for complete samples.

## Tests and missing miners

```bash
cargo test --all-targets
```

Tests that need real miner binaries (`quip-cpu-sa` and related names from the registry) **fail** when those binaries are absent. They do not skip.

To download released miner binaries into `miners/` (and verify each one runs on this host):

```bash
./tools/fetch-miners.sh
export PATH="$(pwd)/miners:${PATH}"
```

The script downloads each miner from its own GitLab package registry. Each repo's version is resolved independently as the newest semver that repo publishes, so a release in one repo does not wait on any other to cut a matching tag. `MINERS_ALLOW_PRERELEASE` defaults to `1` and includes prerelease tags as candidates. Set it to `0` to exclude prereleases entirely. With the flag on, a prerelease of a newer core version wins over an older stable, so `v0.3.1-rc1` beats `v0.3.0`. With it off, prereleases are dropped from candidacy rather than demoted. Neither miner registry publishes a stable tag today, so stable-only will not resolve until one is cut. An inherited `MINERS_TAG` is ignored with a warning, because one tag cannot express a per-repo answer. `DRY_RUN=1` prints the resolved version per repo and the download plan. After download the script probes each binary, and moves failures to `miners/unusable/` so `miners/` on `PATH` holds only binaries that run on this host. The cpu miners are Linux binaries even for `arm64` assets, so macOS moves them to `unusable/`. Metal miners are Mach-O and stay in `miners/`. On macOS the `quip_cpu_sa_conformance` test still needs the opt-out variable below.

To opt out on a machine without a usable `quip-cpu-sa`:

```bash
ISINGMARK_ALLOW_MISSING_MINERS=1 cargo test --all-targets
```

Plot tests need the feature flag:

```bash
ISINGMARK_ALLOW_MISSING_MINERS=1 cargo test --all-targets --features plot
```

## Local quip-miner override

To build against a local checkout of `quip-miner` instead of the pinned git rev:

1. Copy `.cargo/config.toml.example` to `.cargo/config.toml`.
2. Edit the three path entries to your checkout.
3. Build as usual.

`.cargo/config.toml` is gitignored so the committed `Cargo.lock` keeps recording the pinned git rev for CI and other clones.

**Warning:** while that patch file is present, every `cargo` command rewrites `Cargo.lock` to path sources. Before you commit, check that the lock still has three git sources:

```bash
rg -c 'source = "git\+https://gitlab.com/quip.network/quip-miner' Cargo.lock
# expect 3
```

Prefer deleting `.cargo/config.toml` when you are not co-developing upstream.

## Architecture

See `AGENTS.md` for module layout and extension points. See `ANNEALBENCH.md` for mode behaviour, result formats, and design notes. See `docs/datasets.md` for the bundled chain corpora and `docs/backend-notes.md` for measured backend limits.

## License

AGPL-3.0-or-later. See `LICENSE`.
