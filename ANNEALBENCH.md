# IsingMark — Standalone Annealer Benchmark Tool

## Context

Generic, annealer-agnostic benchmarking tool that extracts the problem-generation, solving, and validation loop from quip-protocol-2 and makes it reusable for comparing any Ising-model solver. Users can add new problem types OR new annealers with minimal boilerplate, then run reproducible, comparable benchmarks.

Repo: `isingmark` on glab quip.network. Separate from quip-protocol-2.

### What we're extracting

quip-protocol-2 has rich infrastructure (IsingFeeder, ChaCha8Rng, BaseMiner, Sampler Protocol, IsingModelValidator, energy_utils) but it's tightly coupled to blockchain concerns (nonces, salts, block requirements). IsingMark extracts the pure annealing benchmark loop.

---

## Repository Setup

- **Name**: `isingmark`
- **Host**: glab quip.network
- **Language**: Python (numpy for hot paths)
- **License**: AGPL-3.0-or-later
- **CLI**: `isingmark` via `pyproject.toml` scripts

---

## Package Structure

```
isingmark/
    pyproject.toml
    README.md
    LICENSE
    CLAUDE.md

    isingmark/
        __init__.py
        cli.py                          # Click CLI entry point

        # Core data types
        models.py                       # IsingProblem, AnnealerSolution, BenchmarkConfig

        # RNG (ported from quip-protocol-2)
        chacha8.py                      # Pure Python ChaCha8Rng

        # Problem generation
        problems/
            __init__.py                 # Registry + ProblemGenerator protocol
            random_ising.py             # ChaCha8Rng-based (port from quip)
            planted_solution.py         # Known ground state for validation
            max_cut.py                  # Classical MAX-CUT instances
            feeder.py                   # Background pre-generation (IsingFeeder pattern)

        # Solution validation
        validator.py                    # Energy calc, format check, diversity, satisfaction

        # Annealer adapters
        annealers/
            __init__.py                 # Registry + AnnealerAdapter ABC
            sa_dimod.py                 # dwave-samplers SimulatedAnnealingSampler
            dwave_qpu.py               # D-Wave QPU (optional dep)
            cuda_sa.py                  # CUDA SA (optional dep)

        # Parameter adaptation
        params/
            __init__.py
            adaptor.py                  # ParamsAdaptor ABC + DefaultParamsAdaptor
            sa_params.py                # SA-tuned curves
            qpu_params.py              # QPU-tuned curves

        # Benchmark runners
        runners/
            __init__.py
            throughput.py               # Mode A: problems/min at fixed hardness
            sweep.py                    # Mode B: parameter grid search
            comparative.py              # Mode C: match target energy distribution
            ttt.py                      # Mode D: time-to-target CDF
            scaling.py                  # Mode E: problem size scaling
            reproducibility.py          # Mode F: statistical consistency check

        # Results & reporting
        results/
            __init__.py
            storage.py                  # JSONL read/write + summary JSON
            aggregator.py               # Statistical aggregation
            comparator.py              # Cross-annealer comparison (KS distance)

        # Visualization
        viz/
            __init__.py
            heatmaps.py                 # Parameter sweep heatmaps
            curves.py                   # Time-to-mine, TTT CDFs, SA-vs-Gibbs overlays
            tables.py                   # Text/markdown result tables

    # Example topologies & problem sets
    examples/
        topologies/
            zephyr_9_2.json             # D-Wave Advantage2 topology
            pegasus_16.json             # D-Wave Advantage topology
            chimera_16.json             # D-Wave 2000Q topology
            random_regular_500.json     # Random 3-regular graph, 500 nodes
        configs/
            throughput_sa.yaml          # Example throughput config
            sweep_qpu.yaml              # Example QPU sweep config
            comparative_gibbs.yaml      # Example Gibbs comparison config

    tests/
        test_chacha8.py                 # Cross-language test vectors
        test_random_ising.py            # Problem generation determinism
        test_validator.py               # Energy calculation correctness
        test_models.py                  # Serialization roundtrip
        test_sa_adapter.py              # SA adapter smoke test
        test_throughput_runner.py        # Integration test
        test_comparative.py             # KS distance matching
```

---

## Architecture

### Three Extension Points

1. **ProblemGenerator** — add new problem types
2. **AnnealerAdapter** — add new annealers
3. **ParamsAdaptor** — add annealer-specific parameter tuning

### Data Flow

```
CLI (cli.py)
  │
  ├── parse args, load config
  ├── select ProblemGenerator from registry
  ├── create ProblemFeeder (seeds from config or ChaCha8Rng sequence)
  ├── create AnnealerAdapter
  ├── create ParamsAdaptor
  ├── create SolutionValidator
  │
  └── dispatch to Runner (throughput / sweep / comparative / ttt / scaling)
        │
        ├── Runner spawns worker process(es) via multiprocessing (spawn context)
        │     │
        │     └── Worker loop:
        │           1. Pop IsingProblem from feeder (or generate from seed)
        │           2. ParamsAdaptor.adapt(hardness, problem) → params dict
        │           3. AnnealerAdapter.sample_ising(h, J, **params) → AnnealerSolution
        │           4. SolutionValidator.validate_solution(problem, result) → report
        │           5. Push AnnealerSolution to result_queue
        │
        └── Main process collects results
              │
              ├── results/storage.py writes JSONL + summary JSON
              ├── results/aggregator.py computes statistics
              ├── viz/*.py generates plots (optional, if matplotlib available)
              └── Print summary table to stdout
```

---

## Key Interfaces

### IsingProblem (`models.py`)

```python
@dataclasses.dataclass(frozen=True)
class IsingProblem:
    problem_id: str              # e.g., "random_ising_seed42_0"
    h: dict[int, float]          # Linear biases, keyed by node ID
    J: dict[tuple[int, int], float]  # Quadratic couplings
    nodes: list[int]             # Node list (defines variable ordering)
    edges: list[tuple[int, int]] # Edge list
    problem_type: str            # Registry key (e.g., "random_ising")
    seed: int                    # Seed used to generate this problem
    metadata: dict               # h_values, planted_energy, graph_type, etc.
```

### AnnealerSolution (`models.py`)

```python
@dataclasses.dataclass(frozen=True)
class AnnealerSolution:
    problem_id: str
    annealer_name: str
    params: dict                 # {"num_reads": ..., "num_sweeps": ..., ...}
    wall_time_s: float
    best_energy: float
    mean_energy: float
    std_energy: float
    num_samples: int
    energies: list[float]        # All sample energies
    diversity: float             # Avg pairwise Hamming distance of top-k
    num_below_target: int        # Solutions below energy target (0 if no target)
    satisfaction_rate: float     # Fraction of couplings satisfied in best solution
    hardware_timing: dict        # Annealer-specific timing (e.g., qpu_anneal_time)
    timestamp: float
```

### BenchmarkConfig (`models.py`)

```python
@dataclasses.dataclass
class BenchmarkConfig:
    mode: str                    # "throughput", "sweep", "comparative", "ttt", "scaling"
    annealer_name: str
    problem_type: str
    problem_config: dict         # Problem-type-specific config
    hardness: float              # 0.0 to 1.0
    duration_s: float            # For throughput mode
    num_problems: int
    seeds: list[int]             # Explicit seed list (for reproducibility)
    annealer_config: dict        # Annealer-specific config (device, etc.)
    param_overrides: dict        # Override adapted params
    output_dir: str
```

### ProblemGenerator protocol (`problems/__init__.py`)

```python
class ProblemGenerator(Protocol):
    """Protocol for problem type implementations."""

    @property
    def name(self) -> str:
        """Registry name for this problem type."""
        ...

    def generate(
        self,
        seed: int,
        nodes: list[int],
        edges: list[tuple[int, int]],
        **kwargs,
    ) -> IsingProblem:
        """Generate a single problem instance from a seed."""
        ...

    def known_ground_energy(self, problem: IsingProblem) -> float | None:
        """Return known ground state energy, or None if unknown."""
        ...

# Global registry
_REGISTRY: dict[str, ProblemGenerator] = {}

def register(gen: ProblemGenerator) -> None: ...
def get_generator(name: str) -> ProblemGenerator: ...
```

### AnnealerAdapter ABC (`annealers/__init__.py`)

```python
class AnnealerAdapter(ABC):
    """Base class for annealer integrations."""

    @property
    @abstractmethod
    def name(self) -> str:
        """Human-readable annealer name (e.g., 'dwave-sa', 'cuda-gibbs')."""
        ...

    @abstractmethod
    def sample_ising(
        self,
        h: dict[int, float],
        J: dict[tuple[int, int], float],
        *,
        num_reads: int = 100,
        num_sweeps: int = 1000,
        **kwargs,
    ) -> AnnealerSolution:
        """Run the annealer and return results.

        Implementations must handle timing internally and populate
        wall_time_s and hardware_timing in the returned AnnealerSolution.
        """
        ...

    def setup(self) -> None:
        """One-time initialization (e.g., connect to QPU, init CUDA)."""

    def teardown(self) -> None:
        """Cleanup resources."""
```

### ParamsAdaptor ABC (`params/adaptor.py`)

```python
class ParamsAdaptor(ABC):
    """Maps hardness [0, 1] to annealer parameters."""

    @abstractmethod
    def adapt(
        self,
        hardness: float,
        problem: IsingProblem,
        **kwargs,
    ) -> dict:
        """Produce parameter dict for the given hardness level.

        Returns at minimum: {"num_reads": int, "num_sweeps": int}.
        Annealer-specific keys (e.g., "annealing_time") are added
        by subclasses.
        """
        ...


class DefaultParamsAdaptor(ParamsAdaptor):
    """Linear interpolation between min/max bounds."""

    def __init__(
        self,
        min_sweeps: int = 64,
        max_sweeps: int = 4096,
        min_reads: int = 64,
        max_reads: int = 512,
    ): ...

    def adapt(self, hardness: float, problem: IsingProblem, **kwargs) -> dict:
        """Linear interpolation: param = min + hardness * (max - min)."""
        ...


class QPUParamsAdaptor(ParamsAdaptor):
    """QPU-specific: annealing_time replaces num_sweeps."""

    def __init__(
        self,
        min_annealing_time: float = 1.0,
        max_annealing_time: float = 100.0,
        min_reads: int = 128,
        max_reads: int = 1024,
    ): ...

    def adapt(self, hardness: float, problem: IsingProblem, **kwargs) -> dict:
        """Returns {"num_reads": ..., "annealing_time": ...}."""
        ...
```

### SolutionValidator (`validator.py`)

```python
class SolutionValidator:
    """Validates Ising solutions and computes quality metrics."""

    @staticmethod
    def compute_energy(
        spins: list[int],
        h: dict[int, float],
        J: dict[tuple[int, int], float],
        nodes: list[int],
    ) -> float:
        """Compute Ising energy: E = Σ h_i·s_i + Σ J_ij·s_i·s_j"""
        ...

    @staticmethod
    def validate_format(spins: list[int], num_nodes: int) -> list[str]:
        """Check length and spin values in {-1, +1}."""
        ...

    @staticmethod
    def compute_diversity(solutions: list[list[int]]) -> float:
        """Average normalized symmetric Hamming distance."""
        ...

    @staticmethod
    def coupling_satisfaction_rate(
        spins: list[int],
        J: dict[tuple[int, int], float],
        nodes: list[int],
    ) -> float:
        """Fraction of couplings contributing negative energy."""
        ...

    @staticmethod
    def validate_solution(
        problem: IsingProblem,
        result: AnnealerSolution,
        energy_target: float | None = None,
    ) -> dict:
        """Full validation returning structured report."""
        ...
```

### ProblemFeeder (`problems/feeder.py`)

Adapted from quip-protocol-2's `IsingFeeder`, but simplified to seed-based iteration
without blockchain provenance. Uses `ProcessPoolExecutor` with spawn context.

```python
class ProblemFeeder:
    """Background pre-generator of IsingProblems."""

    def __init__(
        self,
        generator: ProblemGenerator,
        nodes: list[int],
        edges: list[tuple[int, int]],
        seeds: Iterable[int],
        buffer_size: int = 8,
        max_workers: int = 2,
        **generator_kwargs,
    ): ...

    def __iter__(self) -> Iterator[IsingProblem]: ...
    def __next__(self) -> IsingProblem: ...
    def pop(self) -> IsingProblem: ...
    def pop_blocking(self) -> IsingProblem: ...
    def try_pop(self) -> IsingProblem | None: ...
    def stop(self) -> None: ...
```

---

## Benchmark Modes

### A. Throughput (`runners/throughput.py`)

Replicates the 90m test. Run annealer for a fixed duration, measure:

- Problems solved per minute
- Energy distribution (min, mean, std)
- Diversity statistics
- Time-per-problem distribution

```python
def run_throughput(
    config: BenchmarkConfig,
    annealer: AnnealerAdapter,
    params_adaptor: ParamsAdaptor,
    feeder: ProblemFeeder,
    validator: SolutionValidator,
) -> list[AnnealerSolution]:
    """Mine problems for config.duration_s, return all solutions."""
    ...
```

### B. Parameter Sweep (`runners/sweep.py`)

Replicates qpu_results analysis. Grid search over parameter combinations:

- Output: heatmaps (num_reads x num_sweeps/annealing_time -> best_energy)
- Line plots (energy vs num_reads, lines per sweep count)
- Summary comparison table

```python
def run_sweep(
    config: BenchmarkConfig,
    annealer: AnnealerAdapter,
    param_grid: dict[str, list],  # e.g., {"num_reads": [64,128,256], "num_sweeps": [100,500,1000]}
    problems: list[IsingProblem],
    validator: SolutionValidator,
) -> list[AnnealerSolution]:
    """Grid search over parameter combinations."""
    ...
```

### C. Comparative / Energy Distribution Match (`runners/comparative.py`)

Given target results from annealer A, find the hardness setting for annealer B that
best matches annealer A's energy distribution. Uses **Kolmogorov-Smirnov distance** as
the metric for binary search over hardness [0, 1].

Steps:

1. Load target results (energy CDF from annealer A)
2. Binary search: for each candidate hardness h, run annealer B, compute KS distance to target CDF
3. Converge on hardness that minimizes KS distance
4. Report: matched hardness, final KS distance, overlay CDF plot

```python
def run_comparative(
    config: BenchmarkConfig,
    annealer: AnnealerAdapter,
    target_results: list[AnnealerSolution],  # From another annealer
    params_adaptor: ParamsAdaptor,
    problems: list[IsingProblem],
    validator: SolutionValidator,
) -> tuple[list[AnnealerSolution], float]:
    """Binary search for hardness that matches target energy distribution.

    Returns (results, matched_hardness).
    """
    ...
```

### D. Time-to-Target (`runners/ttt.py`)

Standard academic metric (Rønnow et al. 2014). For each energy target:

- Run N independent trials
- Record wall time to first reach target energy
- Produce TTT cumulative distribution plot
- Report median TTT, 99th percentile TTT

```python
def run_ttt(
    config: BenchmarkConfig,
    annealer: AnnealerAdapter,
    params_adaptor: ParamsAdaptor,
    problems: list[IsingProblem],
    energy_targets: list[float],
    max_time_per_target: float,
    num_trials: int = 100,
) -> dict[float, list[float]]:
    """Time-to-target: for each energy target, list of times to first reach it.

    Returns {energy_target: [time_1, time_2, ..., time_N]}.
    """
    ...
```

### E. Scaling (`runners/scaling.py`)

Fix hardness, vary problem size (node count). Measure how performance degrades:

- Generate random topologies at each size
- Plot: time-to-solution vs problem size
- Plot: best energy / theoretical minimum vs problem size

### F. Reproducibility (`runners/reproducibility.py`)

Run same problems twice with same params, verify statistical consistency:

- Two-sample KS test on energy distributions
- Report p-value and flag if distributions differ significantly

---

## Ising Model Format

### Canonical representation

All problems use Ising SPIN {-1, +1} domain:

- `h: dict[int, float]` — linear biases keyed by node ID
- `J: dict[tuple[int, int], float]` — quadratic couplings

QUBO conversion is the annealer adapter's responsibility if needed. The benchmark
itself never operates in QUBO space.

### Serialization (JSONL)

Each line is one problem or result:

```json
{"problem_id": "random_ising_42_0", "seed": 42, "h": {"0": -1.0, "1": 0.0}, "J": {"(0,1)": 1.0}, "metadata": {}}
```

### Topology format

JSON with explicit node and edge lists:

```json
{"nodes": [0, 1, 2, 3], "edges": [[0, 1], [1, 2], [2, 3], [0, 3]]}
```

D-Wave topologies (Zephyr, Pegasus, Chimera) are shipped as example JSON files,
not as a runtime dependency on `dwave-networkx`.

---

## Result Storage

### Per-problem results (JSONL)

`results_<timestamp>.jsonl` — one line per problem solved:

```json
{"problem_id": "random_ising_42_0", "annealer_name": "sa-dimod", "params": {"num_reads": 256, "num_sweeps": 1024}, "wall_time_s": 1.23, "best_energy": -14892.0, "mean_energy": -14650.3, "std_energy": 102.5, "num_samples": 256, "diversity": 0.312, "num_below_target": 12, "satisfaction_rate": 0.734, "hardware_timing": {}, "timestamp": 1718000000.0}
```

### Summary (JSON)

`summary_<timestamp>.json`:

```json
{
  "benchmark_mode": "throughput",
  "annealer_name": "sa-dimod",
  "problem_type": "random_ising",
  "hardness": 0.5,
  "topology": {"num_nodes": 4580, "num_edges": 41567, "name": "zephyr_9_2"},
  "total_problems": 100,
  "total_time_s": 600.0,
  "problems_per_minute": 10.0,
  "energy_stats": {"min": -15100.0, "max": -14300.0, "mean": -14720.0, "std": 180.0},
  "diversity_stats": {"min": 0.21, "max": 0.45, "mean": 0.31},
  "timing_stats": {"min_s": 0.8, "max_s": 8.2, "mean_s": 3.1},
  "config": {},
  "hardware": {"platform": "linux", "cpu": "...", "gpu": "...", "ram_gb": 64}
}
```

---

## CLI Design

```
isingmark <mode> [options]

Modes:
  throughput       Run annealer for duration, measure problems/min
  sweep            Grid search over parameters
  comparative      Match target energy distribution via hardness search
  ttt              Time-to-target cumulative distributions
  scaling          Vary problem size, measure degradation
  reproducibility  Statistical consistency check
  compare          Compare two result files side-by-side

Common options:
  --annealer NAME        sa-dimod | dwave-qpu | cuda-sa | ...
  --problem-type TYPE    random_ising | planted | max_cut
  --hardness FLOAT       0.0-1.0 (default: 0.5)
  --num-problems INT     Number of problems (default: 100)
  --seed INT             Base seed for deterministic generation
  --topology FILE        Topology JSON file
  --output-dir DIR       Output directory
  --workers INT          Number of worker processes
  --device STR           Device specifier (e.g., "cuda:0", "qpu")
  -v, --verbose

Mode-specific options:
  throughput:
    --duration DURATION    Duration string: 30s, 5m, 2h (default: 10m)

  sweep:
    --param-grid JSON      Parameter grid as JSON string
                           e.g., '{"num_reads": [64,128,256], "num_sweeps": [100,500,1000]}'

  comparative:
    --target-file FILE     JSONL result file from reference annealer

  ttt:
    --energy-targets LIST  Comma-separated energy targets
    --num-trials INT       Trials per target (default: 100)
    --max-time FLOAT       Max seconds per trial (default: 300)

  scaling:
    --sizes LIST           Comma-separated problem sizes (node counts)

  compare:
    --file-a FILE          First result file
    --file-b FILE          Second result file
    --plot                 Generate comparison plots
```

### Example usage

```bash
# Throughput benchmark with SA on Advantage2 topology
isingmark throughput --annealer sa-dimod --hardness 0.5 --duration 90m \
  --topology examples/topologies/zephyr_9_2.json

# QPU parameter sweep
isingmark sweep --annealer dwave-qpu \
  --param-grid '{"num_reads": [128,256,512], "annealing_time": [1,5,20,100]}' \
  --topology examples/topologies/zephyr_9_2.json

# Find hardness to match QPU results with CUDA SA
isingmark comparative --annealer cuda-sa --target-file qpu_results.jsonl

# Time-to-target at multiple energy levels
isingmark ttt --annealer sa-dimod \
  --energy-targets "-14600,-14800,-15000,-15200" --num-trials 50

# Compare two result sets
isingmark compare --file-a sa_results.jsonl --file-b qpu_results.jsonl --plot
```

---

## Problem Types

### random_ising (default)

Port of `generate_ising_model_from_nonce` from quip-protocol-2. Uses ChaCha8Rng seeded
from a u64 seed. h in {-1, 0, +1}, J in {-1, +1}. Parameterized by topology and h_values.

This is the most important type — it produces problems identical to those used in
quip mining when given the same seed, enabling direct comparison.

### planted_solution

Generate a problem with a known ground state. Useful for validating that an annealer
can actually find optima. Plant a random spin configuration, then set J values to prefer
that configuration with a controllable frustration fraction (higher frustration = harder).

### max_cut

Classical NP-hard benchmark. h=0 everywhere, J in {-1, +1} on a specified graph
structure (random regular, Erdos-Renyi, etc.). Well-studied in the literature, enables
comparison with published results.

---

## How to Add a New Problem Type

1. Create `isingmark/problems/my_problem.py`
2. Implement the `ProblemGenerator` protocol:
   ```python
   class MyProblemGenerator:
       @property
       def name(self) -> str:
           return "my_problem"

       def generate(self, seed, nodes, edges, **kwargs) -> IsingProblem:
           # Generate h, J from seed
           ...
           return IsingProblem(
               problem_id=f"my_problem_{seed}",
               h=h, J=J, nodes=nodes, edges=edges,
               problem_type=self.name, seed=seed, metadata={},
           )

       def known_ground_energy(self, problem) -> float | None:
           return None  # Or the known energy if you planted a solution
   ```
3. Register in `problems/__init__.py`:
   ```python
   from .my_problem import MyProblemGenerator
   register(MyProblemGenerator())
   ```
4. Use: `isingmark throughput --problem-type my_problem`

## How to Add a New Annealer

1. Create `isingmark/annealers/my_annealer.py`
2. Subclass `AnnealerAdapter`:
   ```python
   class MyAnnealerAdapter(AnnealerAdapter):
       @property
       def name(self) -> str:
           return "my-annealer"

       def sample_ising(self, h, J, *, num_reads=100, num_sweeps=1000, **kwargs):
           start = time.monotonic()
           # ... call your annealer ...
           wall_time = time.monotonic() - start
           return AnnealerSolution(
               problem_id="",  # filled by runner
               annealer_name=self.name,
               params={"num_reads": num_reads, "num_sweeps": num_sweeps},
               wall_time_s=wall_time,
               best_energy=...,
               ...
           )
   ```
3. Optionally create a `ParamsAdaptor` subclass for annealer-specific parameters.
4. Register in `annealers/__init__.py`.

---

## Files to Port from quip-protocol-2

| Source | Destination | Changes |
|--------|------------|---------|
| `shared/chacha8.py` | `isingmark/chacha8.py` | Copy verbatim, keep test vectors |
| `shared/quantum_proof_of_work.py` (`generate_ising_model_from_nonce`) | `isingmark/problems/random_ising.py` | Remove nonce/salt, accept seed directly |
| `shared/energy_utils.py` (`IsingModelValidator`, energy calcs) | `isingmark/validator.py` | Generalize, remove blockchain refs |
| `shared/ising_feeder.py` | `isingmark/problems/feeder.py` | Simplify to seed-based iteration |
| `shared/base_miner.py` (`adapt_parameters`, `energy_to_difficulty`) | `isingmark/params/adaptor.py` | Extract adaptation logic |
| `tools/compare_mining_rates.py` (process spawning) | `isingmark/runners/throughput.py` | Generalize worker loop |
| `tools/benchmark_gibbs_curve.py` (SA vs Gibbs) | `isingmark/runners/comparative.py` | Replace energy sweep with KS-based hardness search |
| `tests/chacha8_test_vectors.json` | `tests/chacha8_test_vectors.json` | Copy verbatim |

---

## Process Architecture

Workers run as separate processes via `multiprocessing` with `spawn` context (avoids
CUDA state inheritance). One long-lived worker per annealer instance, communicating
results via `multiprocessing.Queue`. This mirrors the production mining architecture
in quip-protocol-2.

```
Main Process
  ├── ProblemFeeder (ProcessPoolExecutor, spawn context)
  │     └── Worker pool pre-generates IsingProblems from seeds
  │
  ├── AnnealerWorker[0] (mp.Process, spawn context)
  │     └── Loop: pop problem → adapt params → sample_ising → validate → push result
  ├── AnnealerWorker[1]
  │     └── ...
  └── ResultCollector
        └── Aggregates, writes JSONL, prints progress
```

---

## Implementation Phases

### Phase 1: Core MVP
1. `pyproject.toml` + repo scaffold
2. `models.py` — data types
3. `chacha8.py` — copy from quip
4. `problems/random_ising.py` — port generation logic
5. `validator.py` — energy calc, format validation, diversity
6. `annealers/__init__.py` + `annealers/sa_dimod.py` — first adapter
7. `params/adaptor.py` — default linear interpolation
8. `results/storage.py` — JSONL read/write
9. `runners/throughput.py` — simplest runner
10. `cli.py` — Click CLI with `throughput` subcommand
11. Core tests: chacha8, random_ising, validator, sa_adapter

### Phase 2: Full Benchmarks
12. `problems/feeder.py` — background pre-generation
13. `runners/sweep.py` — parameter grid search
14. `runners/comparative.py` — KS-based hardness matching
15. `runners/ttt.py` — time-to-target
16. `results/aggregator.py` + `results/comparator.py`
17. Tests for each runner

### Phase 3: Annealers & Problem Types
18. `annealers/dwave_qpu.py` — QPU adapter (optional dep)
19. `annealers/cuda_sa.py` — CUDA adapter (optional dep)
20. `problems/planted_solution.py` — known ground state
21. `problems/max_cut.py` — MAX-CUT instances
22. `params/sa_params.py` + `params/qpu_params.py` — tuned curves
23. Example topologies in `examples/topologies/`

### Phase 4: Visualization & Polish
24. `viz/heatmaps.py` — sweep result heatmaps
25. `viz/curves.py` — TTT CDFs, time-to-mine, SA-vs-Gibbs overlays
26. `viz/tables.py` — text/markdown tables
27. `runners/scaling.py` + `runners/reproducibility.py`
28. `compare` CLI subcommand
29. README, example configs, CI

---

## Verification Plan

1. **ChaCha8 determinism**: Run test vectors from `chacha8_test_vectors.json`, verify byte-identical output to Rust implementation
2. **Problem generation**: Generate problems with same seeds as quip-protocol-2, verify identical h/J values
3. **Energy calculation**: Validate against known solutions from existing test suite
4. **SA adapter smoke test**: Generate 10 problems, solve with SA, verify energies are negative and solutions are valid
5. **Throughput runner**: Run 1-minute throughput test, verify result JSONL is valid and statistics are reasonable
6. **Comparative mode**: Generate SA results at hardness 0.3, then use comparative mode to recover hardness ~0.3 from those results
7. **Full integration**: Run all benchmark modes on Zephyr(9,2) topology with SA, verify all outputs
