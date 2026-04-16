# IsingMark Design Spec

## Purpose

Standalone, annealer-agnostic benchmarking tool for Ising-model solvers. Extracted
from quip-protocol-2's tightly-coupled blockchain infrastructure into a reusable
benchmark loop: generate problem, solve, validate, report.

## Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| CLI framework | Click | User preference; groups map cleanly to benchmark modes |
| Concurrency | `multiprocessing` (spawn context) | User requirement; avoids GIL; mirrors quip-protocol-2 production architecture; safe for CUDA |
| Progress display | `click.progressbar` or `tqdm` | User requirement; per-problem progress in throughput/sweep/ttt modes |
| RNG | Pure-Python ChaCha8 | Byte-identical to quip-protocol-2 Rust impl; enables cross-language reproducibility |
| Ising domain | Spin {-1, +1} only | QUBO conversion is adapter responsibility |
| Storage | JSONL (per-problem) + JSON (summary) | Append-friendly, diffable, streamable |
| Viz dependency | Optional (matplotlib) | Core runs without it; viz modules guard imports |

## Architecture

Three extension points — each is a registry:

1. **ProblemGenerator** (Protocol) — `problems/` directory
2. **AnnealerAdapter** (ABC) — `annealers/` directory
3. **ParamsAdaptor** (ABC) — `params/` directory

### Data Flow

```
CLI (click groups/commands)
  -> load config -> resolve generator/annealer/adaptor from registries
  -> ProblemFeeder (ProcessPoolExecutor, spawn) pre-generates problems
  -> Runner dispatches to AnnealerWorker processes (mp.Process, spawn)
     -> Worker: pop problem -> adapt params -> sample_ising -> validate -> push result
  -> Main: collect results -> JSONL + summary JSON -> optional viz -> stdout table
```

### Process Architecture

- **ProblemFeeder**: `ProcessPoolExecutor` with spawn context, buffer_size=8
- **AnnealerWorkers**: `mp.Process` with spawn context, one per `--workers`
- **ResultCollector**: Main process aggregates from `mp.Queue`
- Progress bars update in the main process from queue events

## Components

### Core (Phase 1)

| File | Responsibility |
|------|---------------|
| `models.py` | `IsingProblem`, `AnnealerSolution`, `BenchmarkConfig` frozen dataclasses |
| `chacha8.py` | Verbatim port from quip-protocol-2; ChaCha8Rng with test vectors |
| `problems/random_ising.py` | Port of `generate_ising_model_from_nonce`; seed replaces nonce/salt |
| `validator.py` | Energy calc, format check, diversity, coupling satisfaction |
| `annealers/sa_dimod.py` | dwave-samplers SimulatedAnnealingSampler wrapper |
| `params/adaptor.py` | `DefaultParamsAdaptor` (linear interpolation min/max) |
| `results/storage.py` | JSONL append + summary JSON write |
| `runners/throughput.py` | Fixed-duration benchmark with progress bar |
| `cli.py` | Click entry point, `throughput` subcommand |

### Full Benchmarks (Phase 2)

| File | Responsibility |
|------|---------------|
| `problems/feeder.py` | Background pre-generation with ProcessPoolExecutor |
| `runners/sweep.py` | Parameter grid search with progress over grid cells |
| `runners/comparative.py` | KS-distance binary search over hardness |
| `runners/ttt.py` | Time-to-target CDF with progress per trial |
| `results/aggregator.py` | Statistical aggregation (min/max/mean/std/percentiles) |
| `results/comparator.py` | Cross-annealer KS distance comparison |

### Annealers & Problem Types (Phase 3)

| File | Responsibility |
|------|---------------|
| `annealers/dwave_qpu.py` | D-Wave QPU adapter (optional dep) |
| `annealers/cuda_sa.py` | CUDA SA adapter (optional dep) |
| `problems/planted_solution.py` | Known ground state with tunable frustration |
| `problems/max_cut.py` | MAX-CUT on configurable graph structures |
| `params/sa_params.py` | SA-tuned parameter curves |
| `params/qpu_params.py` | QPU-tuned parameter curves |

### Visualization & Polish (Phase 4)

| File | Responsibility |
|------|---------------|
| `viz/heatmaps.py` | Sweep result heatmaps (matplotlib) |
| `viz/curves.py` | TTT CDFs, time-to-mine, SA-vs-Gibbs overlays |
| `viz/tables.py` | Text/markdown result tables |
| `runners/scaling.py` | Problem size scaling analysis |
| `runners/reproducibility.py` | Statistical consistency (two-sample KS) |

## Interfaces

All interfaces are defined in ANNEALBENCH.md sections "Key Interfaces" — no
changes needed. Key points:

- `IsingProblem` and `AnnealerSolution` are frozen dataclasses
- `ProblemGenerator` is a `Protocol` (structural typing, no inheritance required)
- `AnnealerAdapter` is an `ABC` (explicit inheritance, setup/teardown lifecycle)
- `ParamsAdaptor` is an `ABC` with `adapt(hardness, problem) -> dict`

## Progress Bars

- **Throughput**: Time-based progress bar (elapsed / duration)
- **Sweep**: Progress over grid cells (current / total combinations)
- **TTT**: Progress per energy target, then per trial within each target
- **Scaling**: Progress over problem sizes
- **Comparative**: Progress over binary search iterations
- **Reproducibility**: Progress over problem pairs

Implementation: `click.progressbar` for simple cases. For multiprocessing
runners where workers report asynchronously, main process reads from result
queue and updates a counter-based progress bar.

## Testing Strategy

- **ChaCha8**: Cross-language test vectors (JSON fixtures from quip-protocol-2)
- **random_ising**: Generate with known seeds, verify identical h/J to quip-protocol-2
- **Validator**: Known spin configs with hand-computed energies
- **SA adapter**: Smoke test — solve 10 problems, verify valid negative energies
- **Throughput**: 30s integration test, verify JSONL output and statistics
- **Comparative**: Generate SA@0.3, recover ~0.3 via comparative mode

## Dependencies

```toml
[project]
requires-python = ">=3.11"
dependencies = [
    "numpy",
    "click",
    "dimod",
    "dwave-samplers",
]

[project.optional-dependencies]
viz = ["matplotlib"]
dwave = ["dwave-system"]
cuda = []  # placeholder for CUDA SA bindings
dev = ["pytest", "scipy"]
```

## File Porting Map

| quip-protocol-2 source | isingmark destination | Changes |
|------------------------|----------------------|---------|
| `shared/chacha8.py` | `isingmark/chacha8.py` | Verbatim + test vectors |
| `shared/quantum_proof_of_work.py` | `isingmark/problems/random_ising.py` | Remove nonce/salt, accept seed |
| `shared/energy_utils.py` | `isingmark/validator.py` | Remove blockchain refs |
| `shared/ising_feeder.py` | `isingmark/problems/feeder.py` | Simplify to seed iteration |
| `shared/base_miner.py` | `isingmark/params/adaptor.py` | Extract adaptation curves |
| `tests/chacha8_test_vectors.json` | `tests/chacha8_test_vectors.json` | Verbatim |
