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

The script pins the upstream fetcher to the same `quip-miner` rev as this crate. It auto-detects the package-registry version: the newest semver present in every miner repo the run needs for `MINER_SET` and this host (cpu always; metal on Darwin `auto`; cuda only when `MINER_SET=cpu-cuda`). `MINERS_ALLOW_PRERELEASE` defaults to `1` and includes prerelease tags as candidates; set it to `0` to exclude prereleases entirely (stable-only). When the flag is on, a prerelease of a newer core version wins over an older stable (for example `v0.3.1-rc1` beats `v0.3.0`); when off, prereleases are dropped from candidacy rather than demoted. The flag widens the set; it does not prefer a prerelease over a stable of the same or higher core. Metal currently publishes only a prerelease, so stable-only will not resolve on macOS until metal cuts a stable tag. An inherited `MINERS_TAG` is ignored with a warning. `DRY_RUN=1` prints the resolved version and the repos consulted. After download the script probes each binary; failures move to `miners/unusable/` so `miners/` on `PATH` only has binaries that run on this host. The cpu miners are Linux binaries even for `arm64` assets; on macOS they land in `unusable/`. Metal miners are Mach-O and stay in `miners/`. On macOS the `quip_cpu_sa_conformance` test still needs the opt-out variable below.

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

See `AGENTS.md` for module layout and extension points. See `ANNEALBENCH.md` for mode behaviour, result formats, and design notes.

## License

AGPL-3.0-or-later. See `LICENSE`.
