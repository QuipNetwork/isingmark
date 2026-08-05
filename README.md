# IsingMark

Annealer-agnostic Ising benchmark CLI. Spawns a quip miner binary, feeds consensus-identical problems, and writes JSONL records plus a summary JSON. Does not join a chain.

## Install and build

Requires Rust 1.97.1 (`rust-toolchain.toml`).

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

Two batches on the same problem sequence; two-sample KS test.

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

To opt out on a machine without miners:

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
