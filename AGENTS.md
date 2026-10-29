# Agent Instructions

IsingMark is a Rust CLI that benchmarks quip Ising miner binaries. It acts as a coordinator. It spawns a miner over a Unix socket and feeds it consensus-identical problems, then records the results as JSONL plus a summary JSON. It does not join a chain and does not submit work.

## Build

Toolchain: Rust 1.97.1 (`rust-toolchain.toml`). No `protoc` is needed: the published `quip-proto` crate ships its generated bindings.

```bash
cargo build
cargo build --release
cargo build --features plot   # optional PNG plots via plotters
```

Install the binary on `$PATH`:

```bash
cargo install --path .
```

## Test

```bash
# Default: real miner binaries must be on $PATH, or tests that need them FAIL
cargo test --all-targets

# Download the newest released miners into miners/:
./tools/fetch-miners.sh
export PATH="$(pwd)/miners:${PATH}"

# Local machines without a usable quip-cpu-sa:
ISINGMARK_ALLOW_MISSING_MINERS=1 cargo test --all-targets

# Plot tests (PNG generation):
ISINGMARK_ALLOW_MISSING_MINERS=1 cargo test --all-targets --features plot
```

Without `ISINGMARK_ALLOW_MISSING_MINERS=1`, a missing `quip-cpu-sa` (and related miners) makes those tests fail rather than skip. Set the variable only when you intend to opt out of real-binary coverage.

`tools/fetch-miners.sh` downloads each miner from its own GitLab package registry and checks that every binary can run on this host. Binaries that fail the host probe move to `miners/unusable/` so `miners/` on `PATH` holds only runners that work here. Cpu package assets are Linux ELFs with the arch in the name only, so macOS moves them to `unusable/` and `quip_cpu_sa_conformance` still needs `ISINGMARK_ALLOW_MISSING_MINERS=1`. Metal miners are usable on Darwin. Each repo's version is resolved independently as the newest semver that repo publishes, so a release in one repo does not wait on any other. `MINERS_ALLOW_PRERELEASE` defaults to `1`, which makes prereleases candidates. Setting it to `0` excludes prereleases entirely. With the flag on, a prerelease of a newer core version wins over an older stable, so `v0.3.1-rc1` beats `v0.3.0`. With it off, prereleases are dropped rather than demoted. A repo that publishes no stable tag fails to resolve under `MINERS_ALLOW_PRERELEASE=0`. Pass through `MINER_SET` and `DRY_RUN`. An inherited `MINERS_TAG` is ignored with a warning, because one tag cannot express a per-repo answer. Network-free check: `tools/fetch-miners.sh --select-version TAG...`.

## Lint and format

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features plot -- -D warnings
```

Denied lints include `unsafe_code`, `clippy::panic`, `clippy::unwrap_used`, `clippy::print_stdout`, `clippy::print_stderr`, and `clippy::allow_attributes`. Prefer `#[expect(..., reason = "...")]` over `#[allow(...)]`.

## CLI

```bash
isingmark --help
isingmark throughput --backend cpu-sa --topology-preset smoke --duration 30s
isingmark sweep --backend cpu-sa --topology-preset smoke \
  --param-grid '{"num_reads":[64,128],"num_sweeps":[100,500]}' --num-jobs 5
isingmark ttt --backend cpu-sa --topology-preset smoke \
  --energy-targets "-5,-10" --num-trials 5
isingmark comparative --backend cpu-sa --topology-preset smoke \
  --target-file path/to/records.jsonl
isingmark scaling --backend cpu-sa --sizes 8,16 --num-jobs 5
isingmark reproducibility --backend cpu-sa --topology-preset smoke --num-problems 10
isingmark compare --file-a a.jsonl --file-b b.jsonl
```

Optional PNG plots (requires `--features plot` at build time):

```bash
isingmark sweep ... --plot out/sweep.png
isingmark ttt ... --plot out/ttt.png
isingmark compare --file-a a.jsonl --file-b b.jsonl --plot out/cdfs.png
```

Without the `plot` feature, `--plot` still parses and exits non-zero with a rebuild message.

YAML config files under `examples/configs/` can supply the same fields. Explicit flags override the file. Built-in defaults apply when both are absent.

## Architecture

```
src/
  main.rs, cli.rs     clap subcommands; --config YAML; flags override file
  config.rs           YAML load and merge
  topology.rs         TopologySpec JSON load and topology hash
  problem.rs          seed -> blake3 salt -> derive_nonce -> draw_ising_milli
  corpus.rs           replay chain-harvested nonces -> draw_ising_milli
  backend/            registry + coordinator session loop over UDS
  params.rs           hardness [0,1] -> JobParams per BackendKind
  record.rs           JobRecord JSONL and Summary JSON
  stats/              percentiles, KS distance, two-sample KS p-value
  modes/              throughput, sweep, comparative, ttt, scaling,
                      reproducibility, compare
  report/             text tables; plot.rs behind the plot feature
problems/
  README.md           index of problem families
  sources.json        blake3 manifest of every copied research source
  <family>/README.md  construction, variants, results, limits, provenance
  <family>/params/    generation parameters per cell
  <family>/sources/   byte-identical copies of research sources the family owns
  shared/sources/     copies used by more than one family
corpora/              chain corpora (chain-h0, chain-ternary) plus a
                      frozen-model example per problem cell, except
                      cbfm-p-f17 (no frozen model), sports-break, and
                      paint-shop (own graphs)
evaluation/qblock/    campaign configs, fixtures, and the evidence source list
fixtures/             topology specs; suite-live-2026-09-21.spec.json is the
                      live suite topology
reports/              import-map.md: provenance and destination tickets
```

`tests/campaign_source_inventory.rs` validates `problems/sources.json` against the copied files. Never edit a copied source in place. Copy a new snapshot from the research workspace and update its manifest entry.

Solver deps (versions in `Cargo.toml`): `quip-proto`, `quip-protocol`, and `quip-solver-core`, published on crates.io from `quip.network/quip-solver-core`.

Local override: copy `.cargo/config.toml.example` to `.cargo/config.toml` (gitignored) to patch those crates to a local `quip-solver-core` checkout. While the patch is active, cargo rewrites `Cargo.lock` to path sources. Before commit, confirm the lock is unchanged:

```bash
git diff --stat Cargo.lock
# expect no output
```

Prefer deleting `.cargo/config.toml` when you are not co-developing upstream.

## Extension points

- **Backend**: add a registry entry in `src/backend/registry.rs` pointing at a miner binary that speaks the quip session protocol. New annealers are miner binaries, not in-process adapters.
- **Topology**: ship a JSON `TopologySpec` under `fixtures/` or pass `--topology-file`.
- **Dataset**: ship a JSONL corpus of chain nonces under `corpora/`. Add it to `corpus::BUNDLED` paired with its fixture. Pin that fixture's `topology_hash` in `tests/topology.rs`. A frozen-model example for a problem cell is a JSONL instance file in `corpora/` that `--instances` replays, and it is not listed in `corpus::BUNDLED`. See `docs/datasets.md`.
- **Problem family**: add `problems/<slug>/README.md` and `problems/<slug>/params/`, and add a row to `problems/README.md`. Add a corpus example for each cell with `python -m isingmark_structured.convert`. Then run `cargo test --test corpora_instances`.
- **Mode**: add a module under `src/modes/`, a clap subcommand in `src/cli.rs`, and a branch in `src/main.rs`.
- **Params**: edit `ParamCurve` defaults or supply `param_curve` in YAML for hardness mapping.

## Modes (what each measures)

| Mode | Measures |
|---|---|
| `throughput` | Jobs per minute over a fixed duration at fixed hardness. `--corpus` replays a chain dataset instead of deriving problems from `--seed`; the corpus is finite, so `--duration` becomes an upper bound |
| `sweep` | Grid over `num_reads` x `num_sweeps` or `anneal_time_us` |
| `comparative` | Hardness search that matches a reference energy distribution |
| `ttt` | Time-to-target per energy level |
| `scaling` | Fixed hardness across node counts |
| `reproducibility` | Two-sample KS on two identical-problem batches |
| `compare` | Post-hoc KS on two JSONL result files (no miner spawn) |

Built-in backends: `cpu-sa`, `cpu-gibbs`, `cpu-fsa`, `cpu-msa`, `cpu-sb`, `cpu-bsb`, `cpu-hdsb`, `cpu-hbsb`, `cpu-gbsb`, `cpu-gdsb`, `cpu-tedsb`, `cpu-sbqa`, `cpu-ggdsb`, `cpu-mps`, `cpu-mfa`, `cpu-flatiron`, `cuda`, `cuda-gibbs`, `metal`, `dwave`, `beit`, `cuopt`.

Of the CPU kernels, `cpu-sa`, `cpu-gibbs`, and `cpu-sb` ship as release assets. The other thirteen build behind the `experimental` feature of `quip-solver-cpu` and reach `PATH` only when someone builds them on purpose:

```bash
cargo build --release --features experimental
```

`beit` (`quip-beit-sa`) and `cuopt` (`quip-cuopt-milp`) are not published as release assets, so `tools/fetch-miners.sh` cannot supply them. Build them from `quip.network/ising-solver-beit` (Rust) and `quip.network/ising-solver-nvidia` (Python), then put the binaries on `PATH`. Both need an external resource to run jobs: `beit` needs `BEIT_CUSTOMER_KEY` and bills per sample, `cuopt` needs a self-hosted cuOpt server.

## License

AGPL-3.0-or-later

## Non-Interactive Shell Commands

**ALWAYS use non-interactive flags** with file operations to avoid hanging on confirmation prompts.

Shell commands like `cp`, `mv`, and `rm` may be aliased to include `-i` (interactive) mode on some systems, causing the agent to hang indefinitely waiting for y/n input.

**Use these forms instead:**
```bash
# Force overwrite without prompting
cp -f source dest           # NOT: cp source dest
mv -f source dest           # NOT: mv source dest
rm -f file                  # NOT: rm file

# For recursive operations
rm -rf directory            # NOT: rm -r directory
cp -rf source dest          # NOT: cp -r source dest
```

**Other commands that may prompt:**
- `scp` - use `-o BatchMode=yes` for non-interactive
- `ssh` - use `-o BatchMode=yes` to fail instead of prompting
- `apt-get` - use `-y` flag
- `brew` - use `HOMEBREW_NO_AUTO_UPDATE=1` env var

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:6cd5cc61 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->

<!-- BEGIN BEADS CODEX SETUP: generated by bd setup codex -->
## Beads Issue Tracker

Use Beads (`bd`) for durable task tracking in repositories that include it. Use the `beads` skill at `.agents/skills/beads/SKILL.md` (project install) or `~/.agents/skills/beads/SKILL.md` (global install) for Beads workflow guidance, then use the `bd` CLI for issue operations.

### Quick Reference

```bash
bd ready                # Find available work
bd show <id>            # View issue details
bd update <id> --claim  # Claim work
bd close <id>           # Complete work
bd prime                # Refresh Beads context
```

### Rules

- Use `bd` for all task tracking; do not create markdown TODO lists.
- Run `bd prime` when Beads context is missing or stale. Codex 0.129.0+ can load Beads context automatically through native hooks; use `/hooks` to inspect or toggle them.
- Keep persistent project memory in Beads via `bd remember`; do not create ad hoc memory files.

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.
<!-- END BEADS CODEX SETUP -->
