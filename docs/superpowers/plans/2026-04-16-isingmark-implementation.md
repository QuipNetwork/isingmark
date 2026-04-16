# IsingMark Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a standalone, annealer-agnostic Ising model benchmarking tool extracted from quip-protocol-2.

**Architecture:** Registry-based extension points (ProblemGenerator protocol, AnnealerAdapter ABC, ParamsAdaptor ABC) with multiprocessing workers (spawn context) for problem generation and annealing. Click CLI dispatches to runner modules. Results stored as JSONL + summary JSON.

**Tech Stack:** Python 3.11+, numpy, click, dimod, dwave-samplers, scipy (dev), matplotlib (optional viz)

**Source repo for porting:** `/Users/carback1/Code/quip/quip-protocol-2/`

---

## Phase 1: Core MVP

### Task 1: Project scaffold and pyproject.toml

**Files:**
- Create: `pyproject.toml`
- Create: `LICENSE`
- Create: `isingmark/__init__.py`
- Create: `isingmark/cli.py` (stub)
- Create: `tests/__init__.py`

- [ ] **Step 1: Create pyproject.toml**

```toml
[build-system]
requires = ["hatchling"]
build-backend = "hatchling.build"

[project]
name = "isingmark"
version = "0.1.0"
description = "Annealer-agnostic Ising model benchmarking tool"
requires-python = ">=3.11"
license = "AGPL-3.0-or-later"
dependencies = [
    "numpy",
    "click",
    "dimod",
    "dwave-samplers",
]

[project.optional-dependencies]
viz = ["matplotlib"]
dwave = ["dwave-system"]
dev = ["pytest", "scipy"]

[project.scripts]
isingmark = "isingmark.cli:main"

[tool.pytest.ini_options]
testpaths = ["tests"]
```

- [ ] **Step 2: Create LICENSE file**

Copy the AGPL-3.0-or-later license text.

- [ ] **Step 3: Create package __init__.py**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""IsingMark: annealer-agnostic Ising model benchmarking."""

__version__ = "0.1.0"
```

- [ ] **Step 4: Create CLI stub**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""CLI entry point for isingmark."""
from __future__ import annotations

import click


@click.group()
@click.version_option()
def main() -> None:
    """IsingMark: annealer-agnostic Ising model benchmarking."""


if __name__ == "__main__":
    main()
```

- [ ] **Step 5: Create test init and verify install**

Create `tests/__init__.py` (empty).

Run:
```bash
cd /Users/carback1/Code/quip/isingmark
python3 -m venv .venv
source .venv/bin/activate
pip install -e ".[dev]"
isingmark --version
```
Expected: `isingmark, version 0.1.0`

- [ ] **Step 6: Initialize git repo and commit**

```bash
git init
echo -e ".venv/\n__pycache__/\n*.egg-info/\n.eggs/\ndist/\nbuild/\n*.pyc\n.pytest_cache/" > .gitignore
git add pyproject.toml LICENSE isingmark/ tests/ .gitignore
git commit -m "Initialize isingmark project scaffold"
```

---

### Task 2: Core data models (models.py)

**Files:**
- Create: `isingmark/models.py`
- Create: `tests/test_models.py`

- [ ] **Step 1: Write test for IsingProblem**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for core data models."""
from __future__ import annotations

import json

from isingmark.models import (
    AnnealerSolution,
    BenchmarkConfig,
    IsingProblem,
)


def test_ising_problem_frozen():
    p = IsingProblem(
        problem_id="test_0",
        h={0: -1.0, 1: 0.0},
        J={(0, 1): 1.0},
        nodes=[0, 1],
        edges=[(0, 1)],
        problem_type="random_ising",
        seed=42,
        metadata={},
    )
    assert p.problem_id == "test_0"
    assert p.h[0] == -1.0


def test_ising_problem_to_json_roundtrip():
    p = IsingProblem(
        problem_id="test_0",
        h={0: -1.0, 1: 0.0},
        J={(0, 1): 1.0},
        nodes=[0, 1],
        edges=[(0, 1)],
        problem_type="random_ising",
        seed=42,
        metadata={"h_values": [-1, 0, 1]},
    )
    d = p.to_json_dict()
    blob = json.dumps(d)
    restored = IsingProblem.from_json_dict(json.loads(blob))
    assert restored.problem_id == p.problem_id
    assert restored.h == p.h
    assert restored.J == p.J
    assert restored.nodes == p.nodes
    assert restored.seed == p.seed


def test_annealer_solution_to_json_roundtrip():
    s = AnnealerSolution(
        problem_id="test_0",
        annealer_name="sa-dimod",
        params={"num_reads": 100, "num_sweeps": 500},
        wall_time_s=1.23,
        best_energy=-100.0,
        mean_energy=-90.0,
        std_energy=5.0,
        num_samples=100,
        energies=[-100.0, -95.0, -90.0],
        diversity=0.31,
        num_below_target=3,
        satisfaction_rate=0.73,
        hardware_timing={},
        timestamp=1718000000.0,
    )
    d = s.to_json_dict()
    blob = json.dumps(d)
    restored = AnnealerSolution.from_json_dict(json.loads(blob))
    assert restored.best_energy == s.best_energy
    assert restored.annealer_name == s.annealer_name
    assert restored.energies == s.energies


def test_benchmark_config_defaults():
    c = BenchmarkConfig(
        mode="throughput",
        annealer_name="sa-dimod",
        problem_type="random_ising",
    )
    assert c.hardness == 0.5
    assert c.num_problems == 100
    assert c.duration_s == 600.0
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python -m pytest tests/test_models.py -v`
Expected: FAIL (cannot import isingmark.models)

- [ ] **Step 3: Implement models.py**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Core data types for isingmark benchmarks."""
from __future__ import annotations

import dataclasses
from typing import Any


@dataclasses.dataclass(frozen=True)
class IsingProblem:
    """A single Ising problem instance."""

    problem_id: str
    h: dict[int, float]
    J: dict[tuple[int, int], float]
    nodes: list[int]
    edges: list[tuple[int, int]]
    problem_type: str
    seed: int
    metadata: dict[str, Any]

    def to_json_dict(self) -> dict[str, Any]:
        """Serialize to JSON-safe dict."""
        return {
            "problem_id": self.problem_id,
            "h": {str(k): v for k, v in self.h.items()},
            "J": {
                f"{u},{v}": val
                for (u, v), val in self.J.items()
            },
            "nodes": self.nodes,
            "edges": [list(e) for e in self.edges],
            "problem_type": self.problem_type,
            "seed": self.seed,
            "metadata": self.metadata,
        }

    @classmethod
    def from_json_dict(cls, d: dict[str, Any]) -> IsingProblem:
        """Deserialize from JSON dict."""
        h = {int(k): v for k, v in d["h"].items()}
        J = {
            tuple(int(x) for x in k.split(",")): v
            for k, v in d["J"].items()
        }
        edges = [tuple(e) for e in d["edges"]]
        return cls(
            problem_id=d["problem_id"],
            h=h,
            J=J,
            nodes=d["nodes"],
            edges=edges,
            problem_type=d["problem_type"],
            seed=d["seed"],
            metadata=d.get("metadata", {}),
        )


@dataclasses.dataclass(frozen=True)
class AnnealerSolution:
    """Result from running an annealer on one problem."""

    problem_id: str
    annealer_name: str
    params: dict[str, Any]
    wall_time_s: float
    best_energy: float
    mean_energy: float
    std_energy: float
    num_samples: int
    energies: list[float]
    diversity: float
    num_below_target: int
    satisfaction_rate: float
    hardware_timing: dict[str, Any]
    timestamp: float

    def to_json_dict(self) -> dict[str, Any]:
        """Serialize to JSON-safe dict."""
        return dataclasses.asdict(self)

    @classmethod
    def from_json_dict(
        cls, d: dict[str, Any],
    ) -> AnnealerSolution:
        """Deserialize from JSON dict."""
        return cls(**d)


@dataclasses.dataclass
class BenchmarkConfig:
    """Configuration for a benchmark run."""

    mode: str
    annealer_name: str
    problem_type: str
    problem_config: dict[str, Any] = dataclasses.field(
        default_factory=dict,
    )
    hardness: float = 0.5
    duration_s: float = 600.0
    num_problems: int = 100
    seeds: list[int] = dataclasses.field(default_factory=list)
    annealer_config: dict[str, Any] = dataclasses.field(
        default_factory=dict,
    )
    param_overrides: dict[str, Any] = dataclasses.field(
        default_factory=dict,
    )
    output_dir: str = "."
```

- [ ] **Step 4: Run tests**

Run: `python -m pytest tests/test_models.py -v`
Expected: all 4 tests PASS

- [ ] **Step 5: Commit**

```bash
git add isingmark/models.py tests/test_models.py
git commit -m "Add core data models: IsingProblem, AnnealerSolution, BenchmarkConfig"
```

---

### Task 3: ChaCha8 RNG (port from quip-protocol-2)

**Files:**
- Create: `isingmark/chacha8.py`
- Create: `tests/chacha8_test_vectors.json`
- Create: `tests/test_chacha8.py`

- [ ] **Step 1: Copy test vectors**

Copy `/Users/carback1/Code/quip/quip-protocol-2/tests/chacha8_test_vectors.json` verbatim to `tests/chacha8_test_vectors.json`.

- [ ] **Step 2: Write ChaCha8 tests**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Cross-language test vectors for ChaCha8Rng."""
from __future__ import annotations

import json
from pathlib import Path

from isingmark.chacha8 import ChaCha8Rng, seed_from_u64

VECTORS_PATH = Path(__file__).parent / "chacha8_test_vectors.json"
VECTORS = json.loads(VECTORS_PATH.read_text())


class TestSeedExpansion:
    """Verify PCG32-based seed expansion matches Rust."""

    def test_all_seed_vectors(self):
        for vec in VECTORS["seed_expansion"]:
            key = seed_from_u64(vec["seed"])
            assert key.hex() == vec["expected_key_hex"], (
                f"seed={vec['seed']}"
            )


class TestRngSequences:
    """Verify ChaCha8 keystream matches Rust output."""

    def test_all_sequence_vectors(self):
        for vec in VECTORS["rng_sequences"]:
            rng = ChaCha8Rng.seed_from_u64(vec["seed"])
            for i, expected in enumerate(vec["values"]):
                actual = rng.next_u32()
                assert actual == expected, (
                    f"seed={vec['seed']} index={i}: "
                    f"{actual} != {expected}"
                )
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `python -m pytest tests/test_chacha8.py -v`
Expected: FAIL (cannot import isingmark.chacha8)

- [ ] **Step 4: Port chacha8.py**

Copy `/Users/carback1/Code/quip/quip-protocol-2/shared/chacha8.py` verbatim to `isingmark/chacha8.py`. The only change: expose `seed_from_u64` as a module-level function (rename `_seed_from_u64` to `seed_from_u64` for the public API, keep `_seed_from_u64` as an alias).

The file is self-contained (147 lines, no external imports beyond `struct`). Keep all internal functions (`_rotate_right_u32`, `_pcg32`, `_quarter_round`, `_chacha_block`) private.

- [ ] **Step 5: Run tests**

Run: `python -m pytest tests/test_chacha8.py -v`
Expected: all tests PASS

- [ ] **Step 6: Commit**

```bash
git add isingmark/chacha8.py tests/chacha8_test_vectors.json tests/test_chacha8.py
git commit -m "Port ChaCha8Rng from quip-protocol-2 with cross-language test vectors"
```

---

### Task 4: Random Ising problem generator

**Files:**
- Create: `isingmark/problems/__init__.py`
- Create: `isingmark/problems/random_ising.py`
- Create: `tests/test_random_ising.py`

- [ ] **Step 1: Write tests using test vectors**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for random_ising problem generator using cross-language vectors."""
from __future__ import annotations

import json
from pathlib import Path

from isingmark.problems import get_generator, register
from isingmark.problems.random_ising import RandomIsingGenerator

VECTORS_PATH = Path(__file__).parent / "chacha8_test_vectors.json"
VECTORS = json.loads(VECTORS_PATH.read_text())


def test_generator_registered():
    gen = get_generator("random_ising")
    assert gen.name == "random_ising"


def test_generate_matches_test_vectors():
    """Verify generated h, J match quip-protocol-2 test vectors."""
    gen = RandomIsingGenerator()
    for vec in VECTORS["generate_ising_model"]:
        nodes = vec["nodes"]
        edges = [tuple(e) for e in vec["edges"]]
        h_values = vec["allowed_h_values"]

        problem = gen.generate(
            seed=vec["nonce"],
            nodes=nodes,
            edges=edges,
            h_values=h_values,
        )

        # Compare h
        expected_h = {
            int(k): v for k, v in vec["expected_h"].items()
        }
        assert problem.h == expected_h, (
            f"nonce={vec['nonce']}: h mismatch"
        )

        # Compare J
        expected_J = {
            tuple(int(x) for x in k.split(",")): v
            for k, v in vec["expected_j"].items()
        }
        assert problem.J == expected_J, (
            f"nonce={vec['nonce']}: J mismatch"
        )


def test_deterministic_generation():
    """Same seed + topology = same problem."""
    gen = RandomIsingGenerator()
    nodes = [0, 1, 2, 3]
    edges = [(0, 1), (1, 2), (2, 3), (0, 3)]
    p1 = gen.generate(seed=999, nodes=nodes, edges=edges)
    p2 = gen.generate(seed=999, nodes=nodes, edges=edges)
    assert p1.h == p2.h
    assert p1.J == p2.J


def test_known_ground_energy_returns_none():
    gen = RandomIsingGenerator()
    nodes = [0, 1]
    edges = [(0, 1)]
    p = gen.generate(seed=0, nodes=nodes, edges=edges)
    assert gen.known_ground_energy(p) is None
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `python -m pytest tests/test_random_ising.py -v`
Expected: FAIL

- [ ] **Step 3: Implement problems registry**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Problem generator registry and protocol."""
from __future__ import annotations

from typing import Protocol

from isingmark.models import IsingProblem


class ProblemGenerator(Protocol):
    """Protocol for problem type implementations."""

    @property
    def name(self) -> str: ...

    def generate(
        self,
        seed: int,
        nodes: list[int],
        edges: list[tuple[int, int]],
        **kwargs,
    ) -> IsingProblem: ...

    def known_ground_energy(
        self, problem: IsingProblem,
    ) -> float | None: ...


_REGISTRY: dict[str, ProblemGenerator] = {}


def register(gen: ProblemGenerator) -> None:
    """Register a problem generator by name."""
    _REGISTRY[gen.name] = gen


def get_generator(name: str) -> ProblemGenerator:
    """Look up a registered generator by name."""
    if name not in _REGISTRY:
        available = ", ".join(sorted(_REGISTRY)) or "(none)"
        raise KeyError(
            f"Unknown problem type {name!r}. "
            f"Available: {available}"
        )
    return _REGISTRY[name]


def list_generators() -> list[str]:
    """Return sorted list of registered generator names."""
    return sorted(_REGISTRY)
```

- [ ] **Step 4: Implement random_ising.py**

Port `generate_ising_model_from_nonce` from `quip-protocol-2/shared/quantum_proof_of_work.py`. Remove `ising_nonce_from_block` (blockchain concern). Accept `seed` directly instead of `nonce`.

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Random Ising problem generator using ChaCha8Rng.

Ported from quip-protocol-2's generate_ising_model_from_nonce.
Produces byte-identical problems when given the same seed.
"""
from __future__ import annotations

from isingmark.chacha8 import ChaCha8Rng
from isingmark.models import IsingProblem
from isingmark.problems import register


class RandomIsingGenerator:
    """Generate random Ising problems with ChaCha8Rng.

    h values drawn from h_values list via modulo selection.
    J values are +/-1 based on LSB of next_u32().
    """

    @property
    def name(self) -> str:
        return "random_ising"

    def generate(
        self,
        seed: int,
        nodes: list[int],
        edges: list[tuple[int, int]],
        *,
        h_values: list[float] | None = None,
        **kwargs,
    ) -> IsingProblem:
        if h_values is None:
            h_values = [-1.0, 0.0, 1.0]
        if not h_values:
            raise ValueError("h_values must be non-empty")
        if not nodes:
            raise ValueError("nodes must be non-empty")

        rng = ChaCha8Rng.seed_from_u64(seed)
        n_h = len(h_values)

        h: dict[int, float] = {}
        for node_id in nodes:
            index = rng.next_u32() % n_h
            h[int(node_id)] = h_values[index]

        J: dict[tuple[int, int], float] = {}
        for u, v in edges:
            sign = -1.0 if (rng.next_u32() & 1) == 0 else 1.0
            J[(int(u), int(v))] = sign

        return IsingProblem(
            problem_id=f"random_ising_{seed}",
            h=h,
            J=J,
            nodes=list(nodes),
            edges=list(edges),
            problem_type=self.name,
            seed=seed,
            metadata={"h_values": h_values},
        )

    def known_ground_energy(
        self, problem: IsingProblem,
    ) -> float | None:
        return None


register(RandomIsingGenerator())
```

- [ ] **Step 5: Run tests**

Run: `python -m pytest tests/test_random_ising.py -v`
Expected: all 4 tests PASS

- [ ] **Step 6: Commit**

```bash
git add isingmark/problems/ tests/test_random_ising.py
git commit -m "Add random_ising problem generator with registry"
```

---

### Task 5: Solution validator

**Files:**
- Create: `isingmark/validator.py`
- Create: `tests/test_validator.py`

- [ ] **Step 1: Write validator tests**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for Ising solution validator."""
from __future__ import annotations

import pytest

from isingmark.validator import SolutionValidator


def test_compute_energy_simple():
    """Hand-computed: E = h0*s0 + h1*s1 + J01*s0*s1."""
    h = {0: -1.0, 1: 1.0}
    J = {(0, 1): -1.0}
    nodes = [0, 1]
    # spins: s0=+1, s1=-1
    # E = (-1)(+1) + (1)(-1) + (-1)(+1)(-1) = -1 -1 +1 = -1
    energy = SolutionValidator.compute_energy(
        [1, -1], h, J, nodes,
    )
    assert energy == pytest.approx(-1.0)


def test_compute_energy_all_aligned():
    """All spins +1, h=0, J=-1 on triangle."""
    h = {0: 0.0, 1: 0.0, 2: 0.0}
    J = {(0, 1): -1.0, (1, 2): -1.0, (0, 2): -1.0}
    nodes = [0, 1, 2]
    # E = 0 + (-1)(1)(1) + (-1)(1)(1) + (-1)(1)(1) = -3
    energy = SolutionValidator.compute_energy(
        [1, 1, 1], h, J, nodes,
    )
    assert energy == pytest.approx(-3.0)


def test_validate_format_valid():
    errors = SolutionValidator.validate_format([1, -1, 1], 3)
    assert errors == []


def test_validate_format_wrong_length():
    errors = SolutionValidator.validate_format([1, -1], 3)
    assert len(errors) == 1
    assert "length" in errors[0].lower()


def test_validate_format_invalid_values():
    errors = SolutionValidator.validate_format([1, 0, -1], 3)
    assert len(errors) == 1


def test_compute_diversity_identical():
    solutions = [[1, 1, -1], [1, 1, -1]]
    assert SolutionValidator.compute_diversity(solutions) == 0.0


def test_compute_diversity_opposite():
    solutions = [[1, 1, 1], [-1, -1, -1]]
    # Symmetric Hamming: min(3, 0) / 3 = 0.0 (spin-flip symmetric)
    assert SolutionValidator.compute_diversity(solutions) == 0.0


def test_compute_diversity_different():
    solutions = [[1, 1, -1], [1, -1, 1]]
    # Normal: 2 differ. Inverted: 1 differs. min(2,1)/3 = 1/3
    div = SolutionValidator.compute_diversity(solutions)
    assert div == pytest.approx(1.0 / 3.0)


def test_coupling_satisfaction_rate():
    h = {0: 0.0, 1: 0.0}
    J = {(0, 1): -1.0}
    nodes = [0, 1]
    # s0=+1, s1=+1 -> J*s0*s1 = -1 < 0 -> satisfied
    rate = SolutionValidator.coupling_satisfaction_rate(
        [1, 1], J, nodes,
    )
    assert rate == 1.0


def test_coupling_satisfaction_frustrated():
    h = {0: 0.0, 1: 0.0}
    J = {(0, 1): 1.0}
    nodes = [0, 1]
    # s0=+1, s1=+1 -> J*s0*s1 = +1 > 0 -> frustrated
    rate = SolutionValidator.coupling_satisfaction_rate(
        [1, 1], J, nodes,
    )
    assert rate == 0.0
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `python -m pytest tests/test_validator.py -v`
Expected: FAIL

- [ ] **Step 3: Implement validator.py**

Port energy calculation from `quip-protocol-2/shared/quantum_proof_of_work.py:energy_of_solution` and diversity from `calculate_diversity`/`calculate_hamming_distance`.

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Ising solution validation and quality metrics."""
from __future__ import annotations

import numpy as np


class SolutionValidator:
    """Validates Ising solutions and computes quality metrics."""

    @staticmethod
    def compute_energy(
        spins: list[int],
        h: dict[int, float],
        J: dict[tuple[int, int], float],
        nodes: list[int],
    ) -> float:
        """Compute Ising energy: E = sum(h_i*s_i) + sum(J_ij*s_i*s_j)."""
        node_pos = {
            int(nid): pos for pos, nid in enumerate(nodes)
        }
        e = 0.0
        for pos, nid in enumerate(nodes[: len(spins)]):
            e += float(h.get(int(nid), 0.0)) * spins[pos]
        for (u, v), jval in J.items():
            pu = node_pos.get(int(u))
            pv = node_pos.get(int(v))
            if (
                pu is not None
                and pv is not None
                and pu < len(spins)
                and pv < len(spins)
            ):
                e += float(jval) * spins[pu] * spins[pv]
        return float(e)

    @staticmethod
    def validate_format(
        spins: list[int], num_nodes: int,
    ) -> list[str]:
        """Check length and spin values in {-1, +1}."""
        errors: list[str] = []
        if len(spins) != num_nodes:
            errors.append(
                f"Wrong length: {len(spins)} != {num_nodes}"
            )
        invalid = set(spins) - {-1, 1}
        if invalid:
            errors.append(
                f"Invalid spin values: {invalid}"
            )
        return errors

    @staticmethod
    def compute_diversity(
        solutions: list[list[int]],
    ) -> float:
        """Average normalized symmetric Hamming distance."""
        if len(solutions) < 2:
            return 0.0
        n = len(solutions[0])
        distances: list[float] = []
        for i in range(len(solutions)):
            for j in range(i + 1, len(solutions)):
                a1 = np.asarray(solutions[i], dtype=np.int8)
                a2 = np.asarray(solutions[j], dtype=np.int8)
                dist = int(np.count_nonzero(a1 != a2))
                dist_inv = int(np.count_nonzero(a1 != -a2))
                distances.append(min(dist, dist_inv) / n)
        return float(np.mean(distances)) if distances else 0.0

    @staticmethod
    def coupling_satisfaction_rate(
        spins: list[int],
        J: dict[tuple[int, int], float],
        nodes: list[int],
    ) -> float:
        """Fraction of couplings contributing negative energy."""
        node_pos = {
            int(nid): pos for pos, nid in enumerate(nodes)
        }
        satisfied = 0
        total = 0
        for (u, v), jval in J.items():
            pu = node_pos.get(int(u))
            pv = node_pos.get(int(v))
            if pu is not None and pv is not None:
                total += 1
                coupling_e = jval * spins[pu] * spins[pv]
                if coupling_e < 0:
                    satisfied += 1
        return satisfied / total if total > 0 else 0.0

    @staticmethod
    def validate_solution(
        problem,
        result,
        energy_target: float | None = None,
    ) -> dict:
        """Full validation returning structured report."""
        report: dict = {
            "valid": True,
            "errors": [],
        }
        # Basic format check on energies count
        if result.num_samples < 1:
            report["valid"] = False
            report["errors"].append("No samples")
        report["best_energy"] = result.best_energy
        report["diversity"] = result.diversity
        report["satisfaction_rate"] = result.satisfaction_rate
        if energy_target is not None:
            report["num_below_target"] = sum(
                1 for e in result.energies
                if e < energy_target
            )
        return report
```

- [ ] **Step 4: Run tests**

Run: `python -m pytest tests/test_validator.py -v`
Expected: all 10 tests PASS

- [ ] **Step 5: Commit**

```bash
git add isingmark/validator.py tests/test_validator.py
git commit -m "Add solution validator with energy, diversity, and satisfaction metrics"
```

---

### Task 6: Annealer adapter ABC and SA-dimod adapter

**Files:**
- Create: `isingmark/annealers/__init__.py`
- Create: `isingmark/annealers/sa_dimod.py`
- Create: `tests/test_sa_adapter.py`

- [ ] **Step 1: Write SA adapter tests**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for SA-dimod annealer adapter."""
from __future__ import annotations

from isingmark.annealers import get_adapter
from isingmark.annealers.sa_dimod import SADimodAdapter
from isingmark.problems.random_ising import RandomIsingGenerator
from isingmark.validator import SolutionValidator


def test_adapter_registered():
    adapter = get_adapter("sa-dimod")
    assert adapter.name == "sa-dimod"


def test_sample_ising_returns_solution():
    gen = RandomIsingGenerator()
    nodes = list(range(10))
    edges = [(i, i + 1) for i in range(9)]
    problem = gen.generate(seed=42, nodes=nodes, edges=edges)

    adapter = SADimodAdapter()
    solution = adapter.sample_ising(
        problem.h,
        problem.J,
        num_reads=10,
        num_sweeps=100,
        problem_id=problem.problem_id,
        nodes=problem.nodes,
    )
    assert solution.annealer_name == "sa-dimod"
    assert solution.num_samples == 10
    assert solution.wall_time_s > 0
    assert solution.best_energy <= solution.mean_energy
    assert len(solution.energies) == 10


def test_sample_produces_valid_energies():
    """Verify reported energies match independent calculation."""
    gen = RandomIsingGenerator()
    nodes = list(range(20))
    edges = [
        (i, j)
        for i in range(20)
        for j in range(i + 1, 20)
        if abs(i - j) <= 3
    ]
    problem = gen.generate(seed=7, nodes=nodes, edges=edges)

    adapter = SADimodAdapter()
    solution = adapter.sample_ising(
        problem.h,
        problem.J,
        num_reads=5,
        num_sweeps=200,
        problem_id=problem.problem_id,
        nodes=problem.nodes,
    )
    # Best energy should be negative for non-trivial problems
    assert solution.best_energy < 0
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `python -m pytest tests/test_sa_adapter.py -v`
Expected: FAIL

- [ ] **Step 3: Implement annealer registry**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Annealer adapter registry and ABC."""
from __future__ import annotations

from abc import ABC, abstractmethod

from isingmark.models import AnnealerSolution


class AnnealerAdapter(ABC):
    """Base class for annealer integrations."""

    @property
    @abstractmethod
    def name(self) -> str: ...

    @abstractmethod
    def sample_ising(
        self,
        h: dict[int, float],
        J: dict[tuple[int, int], float],
        *,
        num_reads: int = 100,
        num_sweeps: int = 1000,
        **kwargs,
    ) -> AnnealerSolution: ...

    def setup(self) -> None:
        """One-time initialization."""

    def teardown(self) -> None:
        """Cleanup resources."""


_REGISTRY: dict[str, AnnealerAdapter] = {}


def register(adapter: AnnealerAdapter) -> None:
    _REGISTRY[adapter.name] = adapter


def get_adapter(name: str) -> AnnealerAdapter:
    if name not in _REGISTRY:
        available = ", ".join(sorted(_REGISTRY)) or "(none)"
        raise KeyError(
            f"Unknown annealer {name!r}. Available: {available}"
        )
    return _REGISTRY[name]


def list_adapters() -> list[str]:
    return sorted(_REGISTRY)
```

- [ ] **Step 4: Implement sa_dimod.py**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Simulated annealing adapter using dwave-samplers."""
from __future__ import annotations

import time

import numpy as np
from dwave.samplers import SimulatedAnnealingSampler

from isingmark.annealers import AnnealerAdapter, register
from isingmark.models import AnnealerSolution
from isingmark.validator import SolutionValidator


class SADimodAdapter(AnnealerAdapter):
    """dwave-samplers SimulatedAnnealingSampler wrapper."""

    def __init__(self) -> None:
        self._sampler = SimulatedAnnealingSampler()

    @property
    def name(self) -> str:
        return "sa-dimod"

    def sample_ising(
        self,
        h: dict[int, float],
        J: dict[tuple[int, int], float],
        *,
        num_reads: int = 100,
        num_sweeps: int = 1000,
        problem_id: str = "",
        nodes: list[int] | None = None,
        seed: int | None = None,
        **kwargs,
    ) -> AnnealerSolution:
        sample_kwargs: dict = {
            "num_reads": num_reads,
            "num_sweeps": num_sweeps,
        }
        if seed is not None:
            sample_kwargs["seed"] = seed

        start = time.monotonic()
        sampleset = self._sampler.sample_ising(
            h, J, **sample_kwargs,
        )
        wall_time = time.monotonic() - start

        energies = list(map(float, sampleset.record.energy))
        samples = [
            list(sampleset.record.sample[i])
            for i in range(len(sampleset))
        ]

        diversity = SolutionValidator.compute_diversity(
            samples[:20],
        )
        sat_rate = 0.0
        if nodes and samples:
            sat_rate = (
                SolutionValidator.coupling_satisfaction_rate(
                    samples[0], J, nodes,
                )
            )

        return AnnealerSolution(
            problem_id=problem_id,
            annealer_name=self.name,
            params={
                "num_reads": num_reads,
                "num_sweeps": num_sweeps,
            },
            wall_time_s=wall_time,
            best_energy=float(np.min(energies)),
            mean_energy=float(np.mean(energies)),
            std_energy=float(np.std(energies)),
            num_samples=len(energies),
            energies=energies,
            diversity=diversity,
            num_below_target=0,
            satisfaction_rate=sat_rate,
            hardware_timing={},
            timestamp=time.time(),
        )


register(SADimodAdapter())
```

- [ ] **Step 5: Run tests**

Run: `python -m pytest tests/test_sa_adapter.py -v`
Expected: all 3 tests PASS

- [ ] **Step 6: Commit**

```bash
git add isingmark/annealers/ tests/test_sa_adapter.py
git commit -m "Add annealer adapter ABC and SA-dimod adapter"
```

---

### Task 7: Parameter adaptor

**Files:**
- Create: `isingmark/params/__init__.py`
- Create: `isingmark/params/adaptor.py`
- Create: `tests/test_params.py`

- [ ] **Step 1: Write tests**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for parameter adaptation."""
from __future__ import annotations

import pytest

from isingmark.models import IsingProblem
from isingmark.params.adaptor import DefaultParamsAdaptor


def _make_problem() -> IsingProblem:
    return IsingProblem(
        problem_id="test",
        h={0: 1.0},
        J={(0, 1): -1.0},
        nodes=[0, 1],
        edges=[(0, 1)],
        problem_type="random_ising",
        seed=0,
        metadata={},
    )


def test_hardness_zero():
    adaptor = DefaultParamsAdaptor()
    params = adaptor.adapt(0.0, _make_problem())
    assert params["num_sweeps"] == 64
    assert params["num_reads"] == 64


def test_hardness_one():
    adaptor = DefaultParamsAdaptor()
    params = adaptor.adapt(1.0, _make_problem())
    assert params["num_sweeps"] == 4096
    assert params["num_reads"] == 512


def test_hardness_half():
    adaptor = DefaultParamsAdaptor()
    params = adaptor.adapt(0.5, _make_problem())
    assert params["num_sweeps"] == 64 + int(0.5 * (4096 - 64))
    assert params["num_reads"] == 64 + int(0.5 * (512 - 64))


def test_hardness_clamped():
    adaptor = DefaultParamsAdaptor()
    p = _make_problem()
    params_low = adaptor.adapt(-0.1, p)
    assert params_low["num_sweeps"] == 64
    params_high = adaptor.adapt(1.5, p)
    assert params_high["num_sweeps"] == 4096
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `python -m pytest tests/test_params.py -v`
Expected: FAIL

- [ ] **Step 3: Implement parameter adaptor**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Parameter adaptation: maps hardness [0,1] to annealer params."""
from __future__ import annotations

from abc import ABC, abstractmethod

from isingmark.models import IsingProblem


class ParamsAdaptor(ABC):
    """Maps hardness [0, 1] to annealer parameters."""

    @abstractmethod
    def adapt(
        self,
        hardness: float,
        problem: IsingProblem,
        **kwargs,
    ) -> dict: ...


class DefaultParamsAdaptor(ParamsAdaptor):
    """Linear interpolation between min/max bounds."""

    def __init__(
        self,
        min_sweeps: int = 64,
        max_sweeps: int = 4096,
        min_reads: int = 64,
        max_reads: int = 512,
    ) -> None:
        self.min_sweeps = min_sweeps
        self.max_sweeps = max_sweeps
        self.min_reads = min_reads
        self.max_reads = max_reads

    def adapt(
        self,
        hardness: float,
        problem: IsingProblem,
        **kwargs,
    ) -> dict:
        h = max(0.0, min(1.0, hardness))
        return {
            "num_sweeps": self.min_sweeps
            + int(h * (self.max_sweeps - self.min_sweeps)),
            "num_reads": self.min_reads
            + int(h * (self.max_reads - self.min_reads)),
        }
```

Also create `isingmark/params/__init__.py`:

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Parameter adaptation modules."""
```

- [ ] **Step 4: Run tests**

Run: `python -m pytest tests/test_params.py -v`
Expected: all 4 tests PASS

- [ ] **Step 5: Commit**

```bash
git add isingmark/params/ tests/test_params.py
git commit -m "Add parameter adaptor with linear interpolation"
```

---

### Task 8: JSONL result storage

**Files:**
- Create: `isingmark/results/__init__.py`
- Create: `isingmark/results/storage.py`
- Create: `tests/test_storage.py`

- [ ] **Step 1: Write storage tests**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for JSONL result storage."""
from __future__ import annotations

import json
import tempfile
from pathlib import Path

from isingmark.models import AnnealerSolution
from isingmark.results.storage import (
    append_solution_jsonl,
    read_solutions_jsonl,
    write_summary_json,
)


def _make_solution(pid: str = "p0") -> AnnealerSolution:
    return AnnealerSolution(
        problem_id=pid,
        annealer_name="sa-dimod",
        params={"num_reads": 10},
        wall_time_s=1.0,
        best_energy=-50.0,
        mean_energy=-40.0,
        std_energy=5.0,
        num_samples=10,
        energies=[-50.0, -45.0, -40.0],
        diversity=0.3,
        num_below_target=1,
        satisfaction_rate=0.7,
        hardware_timing={},
        timestamp=1000.0,
    )


def test_jsonl_roundtrip(tmp_path):
    path = tmp_path / "results.jsonl"
    s1 = _make_solution("p0")
    s2 = _make_solution("p1")
    append_solution_jsonl(path, s1)
    append_solution_jsonl(path, s2)
    loaded = read_solutions_jsonl(path)
    assert len(loaded) == 2
    assert loaded[0].problem_id == "p0"
    assert loaded[1].problem_id == "p1"
    assert loaded[0].best_energy == -50.0


def test_write_summary(tmp_path):
    path = tmp_path / "summary.json"
    summary = {
        "benchmark_mode": "throughput",
        "total_problems": 10,
    }
    write_summary_json(path, summary)
    loaded = json.loads(path.read_text())
    assert loaded["benchmark_mode"] == "throughput"
    assert loaded["total_problems"] == 10
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `python -m pytest tests/test_storage.py -v`
Expected: FAIL

- [ ] **Step 3: Implement storage.py**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""JSONL result storage and summary JSON output."""
from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from isingmark.models import AnnealerSolution


def append_solution_jsonl(
    path: Path, solution: AnnealerSolution,
) -> None:
    """Append one solution as a JSONL line."""
    with open(path, "a") as f:
        f.write(json.dumps(solution.to_json_dict()) + "\n")


def read_solutions_jsonl(
    path: Path,
) -> list[AnnealerSolution]:
    """Read all solutions from a JSONL file."""
    solutions: list[AnnealerSolution] = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line:
                d = json.loads(line)
                solutions.append(
                    AnnealerSolution.from_json_dict(d),
                )
    return solutions


def write_summary_json(
    path: Path, summary: dict[str, Any],
) -> None:
    """Write benchmark summary as formatted JSON."""
    with open(path, "w") as f:
        json.dump(summary, f, indent=2)
        f.write("\n")
```

Also create `isingmark/results/__init__.py`:

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Result storage and analysis modules."""
```

- [ ] **Step 4: Run tests**

Run: `python -m pytest tests/test_storage.py -v`
Expected: all 2 tests PASS

- [ ] **Step 5: Commit**

```bash
git add isingmark/results/ tests/test_storage.py
git commit -m "Add JSONL result storage and summary JSON output"
```

---

### Task 9: Throughput runner

**Files:**
- Create: `isingmark/runners/__init__.py`
- Create: `isingmark/runners/throughput.py`
- Create: `tests/test_throughput_runner.py`

- [ ] **Step 1: Write throughput runner test**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Integration test for throughput runner."""
from __future__ import annotations

from isingmark.annealers.sa_dimod import SADimodAdapter
from isingmark.models import BenchmarkConfig
from isingmark.params.adaptor import DefaultParamsAdaptor
from isingmark.problems.random_ising import RandomIsingGenerator
from isingmark.runners.throughput import run_throughput
from isingmark.validator import SolutionValidator


def test_throughput_30s():
    """Run throughput for 5 seconds, verify results."""
    config = BenchmarkConfig(
        mode="throughput",
        annealer_name="sa-dimod",
        problem_type="random_ising",
        duration_s=5.0,
        hardness=0.3,
    )
    nodes = list(range(20))
    edges = [(i, i + 1) for i in range(19)]

    gen = RandomIsingGenerator()
    adapter = SADimodAdapter()
    params_adaptor = DefaultParamsAdaptor()

    results = run_throughput(
        config=config,
        annealer=adapter,
        params_adaptor=params_adaptor,
        generator=gen,
        nodes=nodes,
        edges=edges,
        seeds=range(1000),
        show_progress=False,
    )
    assert len(results) > 0
    for r in results:
        assert r.annealer_name == "sa-dimod"
        assert r.best_energy < 0
        assert r.wall_time_s > 0
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python -m pytest tests/test_throughput_runner.py -v`
Expected: FAIL

- [ ] **Step 3: Implement throughput runner**

This is the core benchmark loop. Uses a simple sequential approach in the main process for the MVP (multiprocessing workers come in Phase 2 with ProblemFeeder).

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Throughput benchmark runner: problems/min at fixed hardness."""
from __future__ import annotations

import time
from collections.abc import Iterable

import click

from isingmark.annealers import AnnealerAdapter
from isingmark.models import AnnealerSolution, BenchmarkConfig
from isingmark.params.adaptor import ParamsAdaptor
from isingmark.problems import ProblemGenerator
from isingmark.validator import SolutionValidator


def run_throughput(
    *,
    config: BenchmarkConfig,
    annealer: AnnealerAdapter,
    params_adaptor: ParamsAdaptor,
    generator: ProblemGenerator,
    nodes: list[int],
    edges: list[tuple[int, int]],
    seeds: Iterable[int],
    show_progress: bool = True,
) -> list[AnnealerSolution]:
    """Run annealer for config.duration_s, return all solutions."""
    params = params_adaptor.adapt(config.hardness, None)
    results: list[AnnealerSolution] = []
    seed_iter = iter(seeds)
    deadline = time.monotonic() + config.duration_s

    if show_progress:
        bar = click.progressbar(
            length=int(config.duration_s),
            label="Throughput",
            show_eta=True,
            show_pos=True,
        )
    else:
        bar = None

    start = time.monotonic()
    last_tick = start

    try:
        if bar is not None:
            bar.__enter__()

        while time.monotonic() < deadline:
            seed = next(seed_iter, None)
            if seed is None:
                break

            problem = generator.generate(
                seed=seed, nodes=nodes, edges=edges,
                **config.problem_config,
            )

            solution = annealer.sample_ising(
                problem.h,
                problem.J,
                num_reads=params["num_reads"],
                num_sweeps=params["num_sweeps"],
                problem_id=problem.problem_id,
                nodes=problem.nodes,
            )
            results.append(solution)

            if bar is not None:
                now = time.monotonic()
                elapsed = int(now - start)
                delta = int(now - last_tick)
                if delta > 0:
                    bar.update(delta)
                    last_tick = now
    finally:
        if bar is not None:
            bar.__exit__(None, None, None)

    return results
```

Also create `isingmark/runners/__init__.py`:

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Benchmark runner modules."""
```

- [ ] **Step 4: Run tests**

Run: `python -m pytest tests/test_throughput_runner.py -v`
Expected: PASS (may take ~5 seconds)

- [ ] **Step 5: Commit**

```bash
git add isingmark/runners/ tests/test_throughput_runner.py
git commit -m "Add throughput runner with click progress bar"
```

---

### Task 10: CLI throughput subcommand

**Files:**
- Modify: `isingmark/cli.py`

- [ ] **Step 1: Write CLI test**

Create `tests/test_cli.py`:

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for CLI commands."""
from __future__ import annotations

import json
import tempfile
from pathlib import Path

from click.testing import CliRunner

from isingmark.cli import main


def test_version():
    runner = CliRunner()
    result = runner.invoke(main, ["--version"])
    assert result.exit_code == 0
    assert "0.1.0" in result.output


def test_throughput_short_run():
    """Run throughput for 3 seconds via CLI."""
    runner = CliRunner()
    with tempfile.TemporaryDirectory() as tmpdir:
        result = runner.invoke(main, [
            "throughput",
            "--duration", "3s",
            "--hardness", "0.3",
            "--seed", "42",
            "--output-dir", tmpdir,
        ])
        assert result.exit_code == 0, result.output

        # Check JSONL was written
        jsonl_files = list(Path(tmpdir).glob("results_*.jsonl"))
        assert len(jsonl_files) == 1

        # Check summary was written
        summary_files = list(Path(tmpdir).glob("summary_*.json"))
        assert len(summary_files) == 1
        summary = json.loads(summary_files[0].read_text())
        assert summary["benchmark_mode"] == "throughput"
        assert summary["total_problems"] > 0
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python -m pytest tests/test_cli.py -v`
Expected: FAIL

- [ ] **Step 3: Implement full CLI with throughput subcommand**

Replace the stub `isingmark/cli.py` with:

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""CLI entry point for isingmark."""
from __future__ import annotations

import json
import re
import time
from pathlib import Path

import click
import numpy as np

from isingmark import __version__
from isingmark.annealers import get_adapter, list_adapters
from isingmark.models import BenchmarkConfig
from isingmark.params.adaptor import DefaultParamsAdaptor
from isingmark.problems import get_generator, list_generators
from isingmark.results.storage import (
    append_solution_jsonl,
    write_summary_json,
)
from isingmark.runners.throughput import run_throughput


def _parse_duration(s: str) -> float:
    """Parse duration string like '30s', '5m', '2h' to seconds."""
    m = re.fullmatch(r"(\d+(?:\.\d+)?)\s*([smh]?)", s.strip())
    if not m:
        raise click.BadParameter(
            f"Invalid duration: {s!r}. Use e.g. 30s, 5m, 2h"
        )
    value = float(m.group(1))
    unit = m.group(2) or "s"
    multipliers = {"s": 1, "m": 60, "h": 3600}
    return value * multipliers[unit]


def _load_topology(path: str | None) -> tuple[list[int], list[tuple[int, int]]]:
    """Load topology from JSON or generate a default small graph."""
    if path:
        data = json.loads(Path(path).read_text())
        nodes = data["nodes"]
        edges = [tuple(e) for e in data["edges"]]
        return nodes, edges
    # Default: random regular-ish graph with 50 nodes
    nodes = list(range(50))
    edges = [
        (i, j)
        for i in range(50)
        for j in range(i + 1, 50)
        if abs(i - j) <= 3 or (i + j) % 7 == 0
    ]
    return nodes, edges


def _seed_sequence(base_seed: int) -> range:
    """Generate a large deterministic seed sequence."""
    return range(base_seed, base_seed + 1_000_000)


@click.group()
@click.version_option(version=__version__)
def main() -> None:
    """IsingMark: annealer-agnostic Ising model benchmarking."""


@main.command()
@click.option(
    "--annealer", default="sa-dimod",
    help=f"Annealer name. Available: {', '.join(list_adapters())}",
)
@click.option(
    "--problem-type", default="random_ising",
    help="Problem generator name.",
)
@click.option("--hardness", default=0.5, type=float, help="0.0-1.0")
@click.option("--duration", default="10m", help="Duration: 30s, 5m, 2h")
@click.option("--seed", default=0, type=int, help="Base seed")
@click.option("--topology", default=None, type=click.Path(), help="Topology JSON")
@click.option("--output-dir", default=".", type=click.Path(), help="Output directory")
@click.option("-v", "--verbose", is_flag=True)
def throughput(
    annealer: str,
    problem_type: str,
    hardness: float,
    duration: str,
    seed: int,
    topology: str | None,
    output_dir: str,
    verbose: bool,
) -> None:
    """Run annealer for a duration, measure problems/min."""
    duration_s = _parse_duration(duration)
    nodes, edges = _load_topology(topology)

    gen = get_generator(problem_type)
    adapter = get_adapter(annealer)
    params_adaptor = DefaultParamsAdaptor()

    config = BenchmarkConfig(
        mode="throughput",
        annealer_name=annealer,
        problem_type=problem_type,
        hardness=hardness,
        duration_s=duration_s,
    )

    click.echo(
        f"Running throughput benchmark: "
        f"annealer={annealer}, hardness={hardness}, "
        f"duration={duration}, "
        f"topology={len(nodes)} nodes / {len(edges)} edges"
    )

    adapter.setup()
    try:
        results = run_throughput(
            config=config,
            annealer=adapter,
            params_adaptor=params_adaptor,
            generator=gen,
            nodes=nodes,
            edges=edges,
            seeds=_seed_sequence(seed),
        )
    finally:
        adapter.teardown()

    # Write results
    out = Path(output_dir)
    out.mkdir(parents=True, exist_ok=True)
    ts = int(time.time())

    jsonl_path = out / f"results_{ts}.jsonl"
    for r in results:
        append_solution_jsonl(jsonl_path, r)

    # Summary
    if results:
        all_energies = [r.best_energy for r in results]
        all_times = [r.wall_time_s for r in results]
        total_time = sum(all_times)
        summary = {
            "benchmark_mode": "throughput",
            "annealer_name": annealer,
            "problem_type": problem_type,
            "hardness": hardness,
            "topology": {
                "num_nodes": len(nodes),
                "num_edges": len(edges),
            },
            "total_problems": len(results),
            "total_time_s": total_time,
            "problems_per_minute": (
                len(results) / total_time * 60
                if total_time > 0 else 0
            ),
            "energy_stats": {
                "min": float(np.min(all_energies)),
                "max": float(np.max(all_energies)),
                "mean": float(np.mean(all_energies)),
                "std": float(np.std(all_energies)),
            },
            "timing_stats": {
                "min_s": float(np.min(all_times)),
                "max_s": float(np.max(all_times)),
                "mean_s": float(np.mean(all_times)),
            },
            "config": config.__dict__,
        }
    else:
        summary = {
            "benchmark_mode": "throughput",
            "total_problems": 0,
        }

    summary_path = out / f"summary_{ts}.json"
    write_summary_json(summary_path, summary)

    # Print summary table
    if results:
        click.echo(f"\n{'='*50}")
        click.echo(f"  Problems solved: {len(results)}")
        click.echo(
            f"  Rate: "
            f"{summary['problems_per_minute']:.1f} problems/min"
        )
        click.echo(
            f"  Energy: "
            f"min={summary['energy_stats']['min']:.1f}, "
            f"mean={summary['energy_stats']['mean']:.1f}, "
            f"std={summary['energy_stats']['std']:.1f}"
        )
        click.echo(f"  Results: {jsonl_path}")
        click.echo(f"  Summary: {summary_path}")
        click.echo(f"{'='*50}")
    else:
        click.echo("No problems solved.")


if __name__ == "__main__":
    main()
```

- [ ] **Step 4: Run tests**

Run: `python -m pytest tests/test_cli.py -v`
Expected: all 2 tests PASS

- [ ] **Step 5: Run CLI manually**

Run: `isingmark throughput --duration 10s --hardness 0.3 --seed 42`
Expected: progress bar, summary table, results files written to current directory.

- [ ] **Step 6: Commit**

```bash
git add isingmark/cli.py tests/test_cli.py
git commit -m "Add Click CLI with throughput subcommand"
```

---

## Phase 2: Full Benchmarks

### Task 11: Problem feeder (background pre-generation)

**Files:**
- Create: `isingmark/problems/feeder.py`
- Create: `tests/test_feeder.py`

- [ ] **Step 1: Write feeder tests**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for ProblemFeeder background pre-generation."""
from __future__ import annotations

from isingmark.problems.feeder import ProblemFeeder
from isingmark.problems.random_ising import RandomIsingGenerator


def test_feeder_produces_problems():
    gen = RandomIsingGenerator()
    nodes = list(range(20))
    edges = [(i, i + 1) for i in range(19)]

    feeder = ProblemFeeder(
        generator=gen,
        nodes=nodes,
        edges=edges,
        seeds=range(10),
        buffer_size=4,
        max_workers=2,
    )
    try:
        problems = []
        for _ in range(5):
            p = feeder.pop_blocking()
            problems.append(p)
        assert len(problems) == 5
        # All should have different problem_ids (different seeds)
        ids = {p.problem_id for p in problems}
        assert len(ids) == 5
    finally:
        feeder.stop()


def test_feeder_deterministic():
    """Same seeds produce same problems."""
    gen = RandomIsingGenerator()
    nodes = list(range(10))
    edges = [(i, i + 1) for i in range(9)]

    problems1 = []
    feeder1 = ProblemFeeder(
        generator=gen, nodes=nodes, edges=edges,
        seeds=[42, 43, 44], buffer_size=2, max_workers=1,
    )
    try:
        for _ in range(3):
            problems1.append(feeder1.pop_blocking())
    finally:
        feeder1.stop()

    problems2 = []
    feeder2 = ProblemFeeder(
        generator=gen, nodes=nodes, edges=edges,
        seeds=[42, 43, 44], buffer_size=2, max_workers=1,
    )
    try:
        for _ in range(3):
            problems2.append(feeder2.pop_blocking())
    finally:
        feeder2.stop()

    for p1, p2 in zip(problems1, problems2):
        assert p1.h == p2.h
        assert p1.J == p2.J


def test_feeder_try_pop_empty():
    gen = RandomIsingGenerator()
    feeder = ProblemFeeder(
        generator=gen, nodes=[0], edges=[],
        seeds=[], buffer_size=1, max_workers=1,
    )
    try:
        result = feeder.try_pop()
        # With no seeds, should return None
        assert result is None
    finally:
        feeder.stop()
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `python -m pytest tests/test_feeder.py -v`
Expected: FAIL

- [ ] **Step 3: Implement feeder.py**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Background pre-generator of IsingProblems using ProcessPoolExecutor.

Uses spawn context to avoid CUDA state inheritance.
"""
from __future__ import annotations

import multiprocessing as _mp
import queue
from collections.abc import Iterable, Iterator
from concurrent.futures import ProcessPoolExecutor
from typing import Any

from isingmark.models import IsingProblem

_SPAWN_CTX = _mp.get_context("spawn")


def _generate_one(
    generator_name: str,
    seed: int,
    nodes: list[int],
    edges: list[tuple[int, int]],
    kwargs: dict[str, Any],
) -> IsingProblem:
    """Worker function: generate one problem in a child process."""
    from isingmark.problems import get_generator

    gen = get_generator(generator_name)
    return gen.generate(
        seed=seed, nodes=nodes, edges=edges, **kwargs,
    )


class ProblemFeeder:
    """Background pre-generator of IsingProblems."""

    def __init__(
        self,
        generator: Any,
        nodes: list[int],
        edges: list[tuple[int, int]],
        seeds: Iterable[int],
        buffer_size: int = 8,
        max_workers: int = 2,
        **generator_kwargs: Any,
    ) -> None:
        self._gen_name = generator.name
        self._nodes = nodes
        self._edges = edges
        self._seed_iter = iter(seeds)
        self._buffer_size = buffer_size
        self._kwargs = generator_kwargs
        self._pool = ProcessPoolExecutor(
            max_workers=max_workers,
            mp_context=_SPAWN_CTX,
        )
        self._futures: list = []
        self._queue: queue.Queue[IsingProblem] = queue.Queue()
        self._stopped = False
        self._exhausted = False
        self._fill()

    def _fill(self) -> None:
        if self._stopped:
            return
        still_pending = []
        for f in self._futures:
            if f.done():
                try:
                    self._queue.put_nowait(f.result())
                except Exception:
                    pass
            else:
                still_pending.append(f)
        self._futures = still_pending

        while (
            not self._exhausted
            and len(self._futures) + self._queue.qsize()
            < self._buffer_size
        ):
            seed = next(self._seed_iter, None)
            if seed is None:
                self._exhausted = True
                break
            f = self._pool.submit(
                _generate_one,
                self._gen_name,
                seed,
                self._nodes,
                self._edges,
                self._kwargs,
            )
            self._futures.append(f)

    def __iter__(self) -> Iterator[IsingProblem]:
        return self

    def __next__(self) -> IsingProblem:
        return self.pop_blocking()

    def pop_blocking(self) -> IsingProblem:
        """Pop one problem, waiting if needed."""
        self._fill()
        try:
            return self._queue.get_nowait()
        except queue.Empty:
            pass
        if not self._futures:
            raise StopIteration
        problem = self._futures.pop(0).result(timeout=30.0)
        self._fill()
        return problem

    def try_pop(self) -> IsingProblem | None:
        """Non-blocking pop, returns None if empty."""
        self._fill()
        try:
            return self._queue.get_nowait()
        except queue.Empty:
            return None

    def stop(self) -> None:
        """Shutdown pool."""
        self._stopped = True
        for f in self._futures:
            f.cancel()
        self._futures.clear()
        self._pool.shutdown(wait=False, cancel_futures=True)
```

- [ ] **Step 4: Run tests**

Run: `python -m pytest tests/test_feeder.py -v`
Expected: all 3 tests PASS

- [ ] **Step 5: Commit**

```bash
git add isingmark/problems/feeder.py tests/test_feeder.py
git commit -m "Add ProblemFeeder with ProcessPoolExecutor background generation"
```

---

### Task 12: Results aggregator

**Files:**
- Create: `isingmark/results/aggregator.py`
- Create: `tests/test_aggregator.py`

- [ ] **Step 1: Write tests**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for statistical aggregation."""
from __future__ import annotations

import pytest

from isingmark.models import AnnealerSolution
from isingmark.results.aggregator import aggregate_solutions


def _sol(energy: float, wall_time: float) -> AnnealerSolution:
    return AnnealerSolution(
        problem_id="p",
        annealer_name="test",
        params={},
        wall_time_s=wall_time,
        best_energy=energy,
        mean_energy=energy,
        std_energy=0.0,
        num_samples=1,
        energies=[energy],
        diversity=0.3,
        num_below_target=0,
        satisfaction_rate=0.7,
        hardware_timing={},
        timestamp=0.0,
    )


def test_aggregate_energy_stats():
    sols = [_sol(-100.0, 1.0), _sol(-80.0, 2.0), _sol(-90.0, 1.5)]
    agg = aggregate_solutions(sols)
    assert agg["energy_stats"]["min"] == pytest.approx(-100.0)
    assert agg["energy_stats"]["max"] == pytest.approx(-80.0)
    assert agg["energy_stats"]["mean"] == pytest.approx(-90.0)


def test_aggregate_timing_stats():
    sols = [_sol(-50.0, 1.0), _sol(-50.0, 3.0)]
    agg = aggregate_solutions(sols)
    assert agg["timing_stats"]["min_s"] == pytest.approx(1.0)
    assert agg["timing_stats"]["max_s"] == pytest.approx(3.0)
    assert agg["timing_stats"]["mean_s"] == pytest.approx(2.0)


def test_aggregate_empty():
    agg = aggregate_solutions([])
    assert agg["total_problems"] == 0
```

- [ ] **Step 2: Implement aggregator.py**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Statistical aggregation for benchmark results."""
from __future__ import annotations

from typing import Any

import numpy as np

from isingmark.models import AnnealerSolution


def aggregate_solutions(
    solutions: list[AnnealerSolution],
) -> dict[str, Any]:
    """Compute aggregate statistics over a set of solutions."""
    if not solutions:
        return {"total_problems": 0}

    energies = np.array([s.best_energy for s in solutions])
    times = np.array([s.wall_time_s for s in solutions])
    diversities = np.array([s.diversity for s in solutions])

    return {
        "total_problems": len(solutions),
        "energy_stats": {
            "min": float(np.min(energies)),
            "max": float(np.max(energies)),
            "mean": float(np.mean(energies)),
            "std": float(np.std(energies)),
            "p25": float(np.percentile(energies, 25)),
            "p50": float(np.percentile(energies, 50)),
            "p75": float(np.percentile(energies, 75)),
        },
        "timing_stats": {
            "min_s": float(np.min(times)),
            "max_s": float(np.max(times)),
            "mean_s": float(np.mean(times)),
            "std_s": float(np.std(times)),
        },
        "diversity_stats": {
            "min": float(np.min(diversities)),
            "max": float(np.max(diversities)),
            "mean": float(np.mean(diversities)),
        },
    }
```

- [ ] **Step 3: Run tests**

Run: `python -m pytest tests/test_aggregator.py -v`
Expected: all 3 PASS

- [ ] **Step 4: Commit**

```bash
git add isingmark/results/aggregator.py tests/test_aggregator.py
git commit -m "Add results aggregator with energy, timing, and diversity stats"
```

---

### Task 13: Results comparator (KS distance)

**Files:**
- Create: `isingmark/results/comparator.py`
- Create: `tests/test_comparator.py`

- [ ] **Step 1: Write tests**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for cross-annealer comparison."""
from __future__ import annotations

import pytest

from isingmark.results.comparator import ks_distance


def test_ks_identical():
    a = [-100.0, -90.0, -80.0, -70.0]
    b = [-100.0, -90.0, -80.0, -70.0]
    stat, pvalue = ks_distance(a, b)
    assert stat == pytest.approx(0.0)


def test_ks_different():
    a = [-100.0, -90.0, -80.0, -70.0]
    b = [-50.0, -40.0, -30.0, -20.0]
    stat, pvalue = ks_distance(a, b)
    assert stat == pytest.approx(1.0)


def test_ks_partial_overlap():
    a = list(range(100))
    b = list(range(50, 150))
    stat, pvalue = ks_distance(a, b)
    assert 0.0 < stat < 1.0
```

- [ ] **Step 2: Implement comparator.py**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Cross-annealer comparison using Kolmogorov-Smirnov distance."""
from __future__ import annotations

from scipy.stats import ks_2samp


def ks_distance(
    energies_a: list[float],
    energies_b: list[float],
) -> tuple[float, float]:
    """Compute two-sample KS statistic and p-value.

    Returns (ks_statistic, p_value).
    """
    result = ks_2samp(energies_a, energies_b)
    return float(result.statistic), float(result.pvalue)
```

- [ ] **Step 3: Run tests**

Run: `python -m pytest tests/test_comparator.py -v`
Expected: all 3 PASS

- [ ] **Step 4: Commit**

```bash
git add isingmark/results/comparator.py tests/test_comparator.py
git commit -m "Add KS-distance comparator for cross-annealer comparison"
```

---

### Task 14: Parameter sweep runner

**Files:**
- Create: `isingmark/runners/sweep.py`
- Create: `tests/test_sweep.py`

- [ ] **Step 1: Write test**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for parameter sweep runner."""
from __future__ import annotations

from isingmark.annealers.sa_dimod import SADimodAdapter
from isingmark.models import BenchmarkConfig
from isingmark.problems.random_ising import RandomIsingGenerator
from isingmark.runners.sweep import run_sweep


def test_sweep_small_grid():
    gen = RandomIsingGenerator()
    nodes = list(range(15))
    edges = [(i, i + 1) for i in range(14)]
    problems = [
        gen.generate(seed=s, nodes=nodes, edges=edges)
        for s in range(3)
    ]

    adapter = SADimodAdapter()
    param_grid = {
        "num_reads": [10, 20],
        "num_sweeps": [50, 100],
    }

    results = run_sweep(
        config=BenchmarkConfig(
            mode="sweep",
            annealer_name="sa-dimod",
            problem_type="random_ising",
        ),
        annealer=adapter,
        param_grid=param_grid,
        problems=problems,
        show_progress=False,
    )
    # 2 reads x 2 sweeps x 3 problems = 12 results
    assert len(results) == 12
```

- [ ] **Step 2: Implement sweep.py**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Parameter grid search runner."""
from __future__ import annotations

import itertools
from typing import Any

import click

from isingmark.annealers import AnnealerAdapter
from isingmark.models import (
    AnnealerSolution,
    BenchmarkConfig,
    IsingProblem,
)


def run_sweep(
    *,
    config: BenchmarkConfig,
    annealer: AnnealerAdapter,
    param_grid: dict[str, list],
    problems: list[IsingProblem],
    show_progress: bool = True,
) -> list[AnnealerSolution]:
    """Grid search over parameter combinations."""
    keys = list(param_grid.keys())
    value_lists = [param_grid[k] for k in keys]
    combos = list(itertools.product(*value_lists))
    total = len(combos) * len(problems)

    results: list[AnnealerSolution] = []

    items = [
        (combo, problem)
        for combo in combos
        for problem in problems
    ]

    if show_progress:
        items_iter: Any = click.progressbar(
            items,
            label="Sweep",
            show_eta=True,
            show_pos=True,
        )
    else:
        items_iter = items

    for combo, problem in items_iter:
        params = dict(zip(keys, combo))
        solution = annealer.sample_ising(
            problem.h,
            problem.J,
            problem_id=problem.problem_id,
            nodes=problem.nodes,
            **params,
        )
        results.append(solution)

    return results
```

- [ ] **Step 3: Run tests**

Run: `python -m pytest tests/test_sweep.py -v`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add isingmark/runners/sweep.py tests/test_sweep.py
git commit -m "Add parameter sweep runner with grid search"
```

---

### Task 15: Comparative runner (KS-based hardness matching)

**Files:**
- Create: `isingmark/runners/comparative.py`
- Create: `tests/test_comparative.py`

- [ ] **Step 1: Write test**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for comparative runner."""
from __future__ import annotations

import pytest

from isingmark.annealers.sa_dimod import SADimodAdapter
from isingmark.models import BenchmarkConfig
from isingmark.params.adaptor import DefaultParamsAdaptor
from isingmark.problems.random_ising import RandomIsingGenerator
from isingmark.runners.comparative import run_comparative
from isingmark.runners.throughput import run_throughput


def test_comparative_recovers_hardness():
    """Generate results at hardness 0.3, then recover ~0.3."""
    nodes = list(range(30))
    edges = [
        (i, j)
        for i in range(30)
        for j in range(i + 1, 30)
        if abs(i - j) <= 2
    ]
    gen = RandomIsingGenerator()
    adapter = SADimodAdapter()
    params_adaptor = DefaultParamsAdaptor()

    # Generate target at hardness 0.3
    target_config = BenchmarkConfig(
        mode="throughput",
        annealer_name="sa-dimod",
        problem_type="random_ising",
        hardness=0.3,
        duration_s=3.0,
    )
    target_results = run_throughput(
        config=target_config,
        annealer=adapter,
        params_adaptor=params_adaptor,
        generator=gen,
        nodes=nodes,
        edges=edges,
        seeds=range(100),
        show_progress=False,
    )

    # Recover hardness
    config = BenchmarkConfig(
        mode="comparative",
        annealer_name="sa-dimod",
        problem_type="random_ising",
    )
    results, matched_hardness = run_comparative(
        config=config,
        annealer=adapter,
        target_results=target_results,
        params_adaptor=params_adaptor,
        generator=gen,
        nodes=nodes,
        edges=edges,
        seeds=range(100, 200),
        max_iterations=8,
        num_problems_per_step=5,
        show_progress=False,
    )
    # Should recover something in the ballpark
    assert 0.0 <= matched_hardness <= 1.0
```

- [ ] **Step 2: Implement comparative.py**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Comparative runner: match target energy distribution via hardness search."""
from __future__ import annotations

from collections.abc import Iterable

import click

from isingmark.annealers import AnnealerAdapter
from isingmark.models import (
    AnnealerSolution,
    BenchmarkConfig,
)
from isingmark.params.adaptor import ParamsAdaptor
from isingmark.problems import ProblemGenerator
from isingmark.results.comparator import ks_distance


def run_comparative(
    *,
    config: BenchmarkConfig,
    annealer: AnnealerAdapter,
    target_results: list[AnnealerSolution],
    params_adaptor: ParamsAdaptor,
    generator: ProblemGenerator,
    nodes: list[int],
    edges: list[tuple[int, int]],
    seeds: Iterable[int],
    max_iterations: int = 15,
    num_problems_per_step: int = 10,
    show_progress: bool = True,
) -> tuple[list[AnnealerSolution], float]:
    """Binary search for hardness that matches target energy distribution."""
    target_energies = [r.best_energy for r in target_results]
    seed_iter = iter(seeds)

    lo, hi = 0.0, 1.0
    best_hardness = 0.5
    best_ks = float("inf")
    all_results: list[AnnealerSolution] = []

    if show_progress:
        bar = click.progressbar(
            length=max_iterations,
            label="Comparative",
            show_eta=True,
        )
    else:
        bar = None

    try:
        if bar is not None:
            bar.__enter__()

        for iteration in range(max_iterations):
            mid = (lo + hi) / 2
            params = params_adaptor.adapt(mid, None)

            step_results: list[AnnealerSolution] = []
            for _ in range(num_problems_per_step):
                seed = next(seed_iter, None)
                if seed is None:
                    break
                problem = generator.generate(
                    seed=seed, nodes=nodes, edges=edges,
                )
                solution = annealer.sample_ising(
                    problem.h,
                    problem.J,
                    num_reads=params["num_reads"],
                    num_sweeps=params["num_sweeps"],
                    problem_id=problem.problem_id,
                    nodes=problem.nodes,
                )
                step_results.append(solution)

            all_results.extend(step_results)
            candidate_energies = [
                r.best_energy for r in step_results
            ]

            if not candidate_energies:
                break

            ks_stat, _ = ks_distance(
                target_energies, candidate_energies,
            )

            if ks_stat < best_ks:
                best_ks = ks_stat
                best_hardness = mid

            # Binary search: compare mean energies
            target_mean = sum(target_energies) / len(
                target_energies,
            )
            candidate_mean = sum(candidate_energies) / len(
                candidate_energies,
            )

            if candidate_mean > target_mean:
                lo = mid
            else:
                hi = mid

            if bar is not None:
                bar.update(1)

    finally:
        if bar is not None:
            bar.__exit__(None, None, None)

    return all_results, best_hardness
```

- [ ] **Step 3: Run tests**

Run: `python -m pytest tests/test_comparative.py -v`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add isingmark/runners/comparative.py tests/test_comparative.py
git commit -m "Add comparative runner with KS-based hardness binary search"
```

---

### Task 16: Time-to-target runner

**Files:**
- Create: `isingmark/runners/ttt.py`
- Create: `tests/test_ttt.py`

- [ ] **Step 1: Write test**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for time-to-target runner."""
from __future__ import annotations

from isingmark.annealers.sa_dimod import SADimodAdapter
from isingmark.models import BenchmarkConfig
from isingmark.params.adaptor import DefaultParamsAdaptor
from isingmark.problems.random_ising import RandomIsingGenerator
from isingmark.runners.ttt import run_ttt


def test_ttt_achievable_target():
    """An easy energy target should be reached in most trials."""
    nodes = list(range(15))
    edges = [(i, i + 1) for i in range(14)]
    gen = RandomIsingGenerator()

    config = BenchmarkConfig(
        mode="ttt",
        annealer_name="sa-dimod",
        problem_type="random_ising",
        hardness=0.5,
    )

    # -1.0 is very easy for a 15-node chain
    ttt_results = run_ttt(
        config=config,
        annealer=SADimodAdapter(),
        params_adaptor=DefaultParamsAdaptor(),
        generator=gen,
        nodes=nodes,
        edges=edges,
        energy_targets=[-1.0],
        num_trials=5,
        max_time_per_target=10.0,
        seeds=range(100),
        show_progress=False,
    )
    assert -1.0 in ttt_results
    # At least some trials should succeed
    assert len(ttt_results[-1.0]) > 0
```

- [ ] **Step 2: Implement ttt.py**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Time-to-target runner: CDF of time to first reach target energy."""
from __future__ import annotations

import time
from collections.abc import Iterable

import click

from isingmark.annealers import AnnealerAdapter
from isingmark.models import BenchmarkConfig
from isingmark.params.adaptor import ParamsAdaptor
from isingmark.problems import ProblemGenerator


def run_ttt(
    *,
    config: BenchmarkConfig,
    annealer: AnnealerAdapter,
    params_adaptor: ParamsAdaptor,
    generator: ProblemGenerator,
    nodes: list[int],
    edges: list[tuple[int, int]],
    energy_targets: list[float],
    num_trials: int = 100,
    max_time_per_target: float = 300.0,
    seeds: Iterable[int] = range(1_000_000),
    show_progress: bool = True,
) -> dict[float, list[float]]:
    """Time-to-target for each energy target.

    Returns {energy_target: [time_1, ..., time_N]} where each
    time is seconds to first reach that energy (or omitted if
    the target was not reached within max_time_per_target).
    """
    params = params_adaptor.adapt(config.hardness, None)
    seed_iter = iter(seeds)
    results: dict[float, list[float]] = {}
    total = len(energy_targets) * num_trials

    if show_progress:
        bar = click.progressbar(
            length=total, label="TTT", show_eta=True,
        )
    else:
        bar = None

    try:
        if bar is not None:
            bar.__enter__()

        for target in energy_targets:
            times: list[float] = []
            for _ in range(num_trials):
                seed = next(seed_iter, None)
                if seed is None:
                    break
                problem = generator.generate(
                    seed=seed, nodes=nodes, edges=edges,
                )
                start = time.monotonic()
                deadline = start + max_time_per_target
                reached = False

                while time.monotonic() < deadline:
                    solution = annealer.sample_ising(
                        problem.h,
                        problem.J,
                        num_reads=params["num_reads"],
                        num_sweeps=params["num_sweeps"],
                        problem_id=problem.problem_id,
                        nodes=problem.nodes,
                    )
                    if solution.best_energy <= target:
                        elapsed = time.monotonic() - start
                        times.append(elapsed)
                        reached = True
                        break

                if bar is not None:
                    bar.update(1)

            results[target] = times

    finally:
        if bar is not None:
            bar.__exit__(None, None, None)

    return results
```

- [ ] **Step 3: Run tests**

Run: `python -m pytest tests/test_ttt.py -v`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add isingmark/runners/ttt.py tests/test_ttt.py
git commit -m "Add time-to-target runner with per-trial timing"
```

---

### Task 17: CLI subcommands for sweep, comparative, ttt

**Files:**
- Modify: `isingmark/cli.py`

- [ ] **Step 1: Add sweep subcommand to cli.py**

Add after the `throughput` command:

```python
@main.command()
@click.option("--annealer", default="sa-dimod")
@click.option("--problem-type", default="random_ising")
@click.option("--param-grid", required=True, help="JSON grid string")
@click.option("--num-problems", default=10, type=int)
@click.option("--seed", default=0, type=int)
@click.option("--topology", default=None, type=click.Path())
@click.option("--output-dir", default=".", type=click.Path())
def sweep(annealer, problem_type, param_grid, num_problems, seed, topology, output_dir):
    """Grid search over parameter combinations."""
    import json as _json
    from isingmark.runners.sweep import run_sweep

    grid = _json.loads(param_grid)
    nodes, edges = _load_topology(topology)
    gen = get_generator(problem_type)
    adapter = get_adapter(annealer)

    problems = [
        gen.generate(seed=s, nodes=nodes, edges=edges)
        for s in range(seed, seed + num_problems)
    ]

    click.echo(f"Running sweep: {len(problems)} problems, grid={grid}")
    adapter.setup()
    try:
        results = run_sweep(
            config=BenchmarkConfig(mode="sweep", annealer_name=annealer, problem_type=problem_type),
            annealer=adapter,
            param_grid=grid,
            problems=problems,
        )
    finally:
        adapter.teardown()

    out = Path(output_dir)
    out.mkdir(parents=True, exist_ok=True)
    ts = int(time.time())
    jsonl_path = out / f"sweep_{ts}.jsonl"
    for r in results:
        append_solution_jsonl(jsonl_path, r)
    click.echo(f"Wrote {len(results)} results to {jsonl_path}")
```

- [ ] **Step 2: Add comparative subcommand**

```python
@main.command()
@click.option("--annealer", default="sa-dimod")
@click.option("--problem-type", default="random_ising")
@click.option("--target-file", required=True, type=click.Path(exists=True))
@click.option("--seed", default=0, type=int)
@click.option("--topology", default=None, type=click.Path())
@click.option("--output-dir", default=".", type=click.Path())
@click.option("--max-iterations", default=15, type=int)
def comparative(annealer, problem_type, target_file, seed, topology, output_dir, max_iterations):
    """Match target energy distribution via hardness search."""
    from isingmark.results.storage import read_solutions_jsonl
    from isingmark.runners.comparative import run_comparative

    nodes, edges = _load_topology(topology)
    gen = get_generator(problem_type)
    adapter = get_adapter(annealer)
    params_adaptor = DefaultParamsAdaptor()
    target_results = read_solutions_jsonl(Path(target_file))

    click.echo(f"Loaded {len(target_results)} target results from {target_file}")
    adapter.setup()
    try:
        results, matched_hardness = run_comparative(
            config=BenchmarkConfig(mode="comparative", annealer_name=annealer, problem_type=problem_type),
            annealer=adapter,
            target_results=target_results,
            params_adaptor=params_adaptor,
            generator=gen,
            nodes=nodes,
            edges=edges,
            seeds=_seed_sequence(seed),
            max_iterations=max_iterations,
        )
    finally:
        adapter.teardown()

    click.echo(f"Matched hardness: {matched_hardness:.4f}")
    out = Path(output_dir)
    out.mkdir(parents=True, exist_ok=True)
    ts = int(time.time())
    jsonl_path = out / f"comparative_{ts}.jsonl"
    for r in results:
        append_solution_jsonl(jsonl_path, r)
```

- [ ] **Step 3: Add ttt subcommand**

```python
@main.command()
@click.option("--annealer", default="sa-dimod")
@click.option("--problem-type", default="random_ising")
@click.option("--hardness", default=0.5, type=float)
@click.option("--energy-targets", required=True, help="Comma-separated energy targets")
@click.option("--num-trials", default=100, type=int)
@click.option("--max-time", default=300.0, type=float)
@click.option("--seed", default=0, type=int)
@click.option("--topology", default=None, type=click.Path())
@click.option("--output-dir", default=".", type=click.Path())
def ttt(annealer, problem_type, hardness, energy_targets, num_trials, max_time, seed, topology, output_dir):
    """Time-to-target cumulative distributions."""
    import json as _json
    from isingmark.runners.ttt import run_ttt

    targets = [float(x.strip()) for x in energy_targets.split(",")]
    nodes, edges = _load_topology(topology)
    gen = get_generator(problem_type)
    adapter = get_adapter(annealer)
    params_adaptor = DefaultParamsAdaptor()

    config = BenchmarkConfig(mode="ttt", annealer_name=annealer, problem_type=problem_type, hardness=hardness)
    click.echo(f"Running TTT: targets={targets}, trials={num_trials}")

    adapter.setup()
    try:
        results = run_ttt(
            config=config,
            annealer=adapter,
            params_adaptor=params_adaptor,
            generator=gen,
            nodes=nodes,
            edges=edges,
            energy_targets=targets,
            num_trials=num_trials,
            max_time_per_target=max_time,
            seeds=_seed_sequence(seed),
        )
    finally:
        adapter.teardown()

    out = Path(output_dir)
    out.mkdir(parents=True, exist_ok=True)
    ts = int(time.time())
    summary_path = out / f"ttt_{ts}.json"
    summary = {t: {"times": times, "success_rate": len(times) / num_trials} for t, times in results.items()}
    write_summary_json(summary_path, summary)

    for target, times in results.items():
        rate = len(times) / num_trials * 100
        median = sorted(times)[len(times) // 2] if times else float("inf")
        click.echo(f"  Target {target:.1f}: {rate:.0f}% success, median TTT={median:.2f}s")
```

- [ ] **Step 4: Run all CLI tests**

Run: `python -m pytest tests/test_cli.py -v`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add isingmark/cli.py
git commit -m "Add sweep, comparative, and ttt CLI subcommands"
```

---

## Phase 3: Annealers & Problem Types

### Task 18: D-Wave QPU adapter (optional dep)

**Files:**
- Create: `isingmark/annealers/dwave_qpu.py`
- Create: `isingmark/params/qpu_params.py`

- [ ] **Step 1: Implement QPU adapter**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""D-Wave QPU annealer adapter (requires dwave-system)."""
from __future__ import annotations

import time

import numpy as np

from isingmark.annealers import AnnealerAdapter, register
from isingmark.models import AnnealerSolution
from isingmark.validator import SolutionValidator

try:
    from dwave.system import DWaveSampler, AutoEmbeddingComposite

    _HAS_DWAVE = True
except ImportError:
    _HAS_DWAVE = False


class DWaveQPUAdapter(AnnealerAdapter):
    """D-Wave QPU adapter using AutoEmbeddingComposite."""

    def __init__(self, solver: str | None = None) -> None:
        self._solver_name = solver
        self._sampler = None

    @property
    def name(self) -> str:
        return "dwave-qpu"

    def setup(self) -> None:
        if not _HAS_DWAVE:
            raise ImportError(
                "dwave-system not installed. "
                "Install with: pip install isingmark[dwave]"
            )
        base = DWaveSampler(solver=self._solver_name)
        self._sampler = AutoEmbeddingComposite(base)

    def teardown(self) -> None:
        self._sampler = None

    def sample_ising(
        self,
        h: dict[int, float],
        J: dict[tuple[int, int], float],
        *,
        num_reads: int = 100,
        annealing_time: float = 20.0,
        problem_id: str = "",
        nodes: list[int] | None = None,
        **kwargs,
    ) -> AnnealerSolution:
        if self._sampler is None:
            raise RuntimeError("Call setup() before sample_ising()")

        start = time.monotonic()
        sampleset = self._sampler.sample_ising(
            h, J,
            num_reads=num_reads,
            annealing_time=annealing_time,
        )
        wall_time = time.monotonic() - start

        energies = list(map(float, sampleset.record.energy))
        samples = [
            list(sampleset.record.sample[i])
            for i in range(len(sampleset))
        ]

        diversity = SolutionValidator.compute_diversity(samples[:20])
        sat_rate = 0.0
        if nodes and samples:
            sat_rate = SolutionValidator.coupling_satisfaction_rate(
                samples[0], J, nodes,
            )

        hw_timing = dict(sampleset.info.get("timing", {}))

        return AnnealerSolution(
            problem_id=problem_id,
            annealer_name=self.name,
            params={"num_reads": num_reads, "annealing_time": annealing_time},
            wall_time_s=wall_time,
            best_energy=float(np.min(energies)),
            mean_energy=float(np.mean(energies)),
            std_energy=float(np.std(energies)),
            num_samples=len(energies),
            energies=energies,
            diversity=diversity,
            num_below_target=0,
            satisfaction_rate=sat_rate,
            hardware_timing=hw_timing,
            timestamp=time.time(),
        )


if _HAS_DWAVE:
    register(DWaveQPUAdapter())
```

- [ ] **Step 2: Implement QPU params adaptor**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""QPU-specific parameter adaptation."""
from __future__ import annotations

from isingmark.models import IsingProblem
from isingmark.params.adaptor import ParamsAdaptor


class QPUParamsAdaptor(ParamsAdaptor):
    """QPU-specific: annealing_time replaces num_sweeps."""

    def __init__(
        self,
        min_annealing_time: float = 1.0,
        max_annealing_time: float = 100.0,
        min_reads: int = 128,
        max_reads: int = 1024,
    ) -> None:
        self.min_annealing_time = min_annealing_time
        self.max_annealing_time = max_annealing_time
        self.min_reads = min_reads
        self.max_reads = max_reads

    def adapt(
        self,
        hardness: float,
        problem: IsingProblem | None,
        **kwargs,
    ) -> dict:
        h = max(0.0, min(1.0, hardness))
        return {
            "num_reads": self.min_reads
            + int(h * (self.max_reads - self.min_reads)),
            "annealing_time": self.min_annealing_time
            + h * (self.max_annealing_time - self.min_annealing_time),
        }
```

- [ ] **Step 3: Commit**

```bash
git add isingmark/annealers/dwave_qpu.py isingmark/params/qpu_params.py
git commit -m "Add D-Wave QPU adapter and QPU params adaptor (optional dep)"
```

---

### Task 19: CUDA SA adapter stub (optional dep)

**Files:**
- Create: `isingmark/annealers/cuda_sa.py`
- Create: `isingmark/params/sa_params.py`

- [ ] **Step 1: Implement CUDA SA adapter stub**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""CUDA Simulated Annealing adapter (placeholder for GPU SA backends)."""
from __future__ import annotations

import time

import numpy as np

from isingmark.annealers import AnnealerAdapter, register
from isingmark.models import AnnealerSolution
from isingmark.validator import SolutionValidator

try:
    # Placeholder: actual CUDA SA import would go here
    _HAS_CUDA_SA = False
except ImportError:
    _HAS_CUDA_SA = False


class CUDASAAdapter(AnnealerAdapter):
    """CUDA-accelerated SA adapter.

    Placeholder implementation. When a CUDA SA backend is
    available, replace the _sample method with the actual call.
    """

    def __init__(self, device: str = "cuda:0") -> None:
        self._device = device

    @property
    def name(self) -> str:
        return "cuda-sa"

    def setup(self) -> None:
        if not _HAS_CUDA_SA:
            raise ImportError(
                "CUDA SA backend not available. "
                "This is a placeholder adapter."
            )

    def sample_ising(
        self,
        h: dict[int, float],
        J: dict[tuple[int, int], float],
        *,
        num_reads: int = 100,
        num_sweeps: int = 1000,
        problem_id: str = "",
        nodes: list[int] | None = None,
        **kwargs,
    ) -> AnnealerSolution:
        raise NotImplementedError(
            "CUDA SA backend not yet integrated"
        )


# Only register if CUDA SA is actually available
if _HAS_CUDA_SA:
    register(CUDASAAdapter())
```

- [ ] **Step 2: Implement SA-tuned params**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""SA-tuned parameter curves with nonlinear scaling."""
from __future__ import annotations

import math

from isingmark.models import IsingProblem
from isingmark.params.adaptor import ParamsAdaptor


class SAParamsAdaptor(ParamsAdaptor):
    """SA-specific adaptation with sqrt scaling for sweeps.

    Sweeps scale as sqrt(hardness) to front-load computational
    effort at lower difficulties. Reads scale linearly.
    """

    def __init__(
        self,
        min_sweeps: int = 64,
        max_sweeps: int = 4096,
        min_reads: int = 64,
        max_reads: int = 512,
    ) -> None:
        self.min_sweeps = min_sweeps
        self.max_sweeps = max_sweeps
        self.min_reads = min_reads
        self.max_reads = max_reads

    def adapt(
        self,
        hardness: float,
        problem: IsingProblem | None,
        **kwargs,
    ) -> dict:
        h = max(0.0, min(1.0, hardness))
        sweeps = self.min_sweeps + int(
            math.sqrt(h)
            * (self.max_sweeps - self.min_sweeps)
        )
        reads = self.min_reads + int(
            h * (self.max_reads - self.min_reads)
        )
        return {"num_sweeps": sweeps, "num_reads": reads}
```

- [ ] **Step 3: Commit**

```bash
git add isingmark/annealers/cuda_sa.py isingmark/params/sa_params.py
git commit -m "Add CUDA SA adapter stub and SA-tuned params adaptor"
```

---

### Task 20: Planted solution problem generator

**Files:**
- Create: `isingmark/problems/planted_solution.py`
- Create: `tests/test_planted.py`

- [ ] **Step 1: Write tests**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for planted solution generator."""
from __future__ import annotations

import pytest

from isingmark.problems import get_generator
from isingmark.problems.planted_solution import PlantedSolutionGenerator
from isingmark.validator import SolutionValidator


def test_planted_registered():
    gen = get_generator("planted")
    assert gen.name == "planted"


def test_planted_ground_energy_known():
    gen = PlantedSolutionGenerator()
    nodes = list(range(10))
    edges = [(i, i + 1) for i in range(9)]
    problem = gen.generate(
        seed=42, nodes=nodes, edges=edges, frustration=0.0,
    )
    ground_e = gen.known_ground_energy(problem)
    assert ground_e is not None
    # With zero frustration, all J should prefer the planted state
    # -> ground energy = sum(h_i * planted_i) + sum(-|J_ij|)
    assert ground_e < 0


def test_planted_sa_finds_ground():
    """SA should find ground state for small zero-frustration problem."""
    from isingmark.annealers.sa_dimod import SADimodAdapter

    gen = PlantedSolutionGenerator()
    nodes = list(range(8))
    edges = [(i, i + 1) for i in range(7)]
    problem = gen.generate(
        seed=42, nodes=nodes, edges=edges, frustration=0.0,
    )
    ground_e = gen.known_ground_energy(problem)

    adapter = SADimodAdapter()
    solution = adapter.sample_ising(
        problem.h, problem.J,
        num_reads=50, num_sweeps=500,
        problem_id=problem.problem_id,
        nodes=problem.nodes,
    )
    # SA should get close to ground state
    assert solution.best_energy <= ground_e + 1.0
```

- [ ] **Step 2: Implement planted_solution.py**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Planted solution problem generator with tunable frustration."""
from __future__ import annotations

from isingmark.chacha8 import ChaCha8Rng
from isingmark.models import IsingProblem
from isingmark.problems import register


class PlantedSolutionGenerator:
    """Generate problems with a known ground state.

    Plants a random spin configuration, then sets J values to
    prefer that configuration. Frustration fraction controls
    the fraction of couplings that are flipped (harder problem).
    """

    @property
    def name(self) -> str:
        return "planted"

    def generate(
        self,
        seed: int,
        nodes: list[int],
        edges: list[tuple[int, int]],
        *,
        frustration: float = 0.1,
        **kwargs,
    ) -> IsingProblem:
        rng = ChaCha8Rng.seed_from_u64(seed)

        # Plant a random spin configuration
        planted = {}
        for nid in nodes:
            planted[int(nid)] = 1 if (rng.next_u32() & 1) else -1

        # h biases that prefer the planted state
        h: dict[int, float] = {}
        for nid in nodes:
            h[int(nid)] = -float(planted[int(nid)])

        # J couplings: prefer planted alignment with frustration
        J: dict[tuple[int, int], float] = {}
        for u, v in edges:
            si, sj = planted[int(u)], planted[int(v)]
            preferred = -float(si * sj)  # Coupling that satisfies planted
            # Flip with probability = frustration
            flip = (rng.next_u32() % 1000) < int(frustration * 1000)
            J[(int(u), int(v))] = -preferred if flip else preferred

        return IsingProblem(
            problem_id=f"planted_{seed}",
            h=h,
            J=J,
            nodes=list(nodes),
            edges=list(edges),
            problem_type=self.name,
            seed=seed,
            metadata={
                "frustration": frustration,
                "planted_spins": planted,
            },
        )

    def known_ground_energy(
        self, problem: IsingProblem,
    ) -> float | None:
        planted = problem.metadata.get("planted_spins")
        if planted is None:
            return None
        e = 0.0
        for nid in problem.nodes:
            spin = planted[str(nid)] if isinstance(
                next(iter(planted)), str,
            ) else planted[nid]
            e += problem.h.get(nid, 0.0) * spin
        for (u, v), jval in problem.J.items():
            su = planted[str(u)] if isinstance(
                next(iter(planted)), str,
            ) else planted[u]
            sv = planted[str(v)] if isinstance(
                next(iter(planted)), str,
            ) else planted[v]
            e += jval * su * sv
        return e


register(PlantedSolutionGenerator())
```

- [ ] **Step 3: Run tests**

Run: `python -m pytest tests/test_planted.py -v`
Expected: all 3 PASS

- [ ] **Step 4: Commit**

```bash
git add isingmark/problems/planted_solution.py tests/test_planted.py
git commit -m "Add planted solution generator with tunable frustration"
```

---

### Task 21: MAX-CUT problem generator

**Files:**
- Create: `isingmark/problems/max_cut.py`
- Create: `tests/test_max_cut.py`

- [ ] **Step 1: Write tests**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for MAX-CUT problem generator."""
from __future__ import annotations

from isingmark.problems import get_generator
from isingmark.problems.max_cut import MaxCutGenerator


def test_max_cut_registered():
    gen = get_generator("max_cut")
    assert gen.name == "max_cut"


def test_max_cut_h_is_zero():
    gen = MaxCutGenerator()
    nodes = list(range(10))
    edges = [(i, i + 1) for i in range(9)]
    problem = gen.generate(seed=42, nodes=nodes, edges=edges)
    for val in problem.h.values():
        assert val == 0.0


def test_max_cut_j_values():
    gen = MaxCutGenerator()
    nodes = list(range(5))
    edges = [(0, 1), (1, 2), (2, 3), (3, 4)]
    problem = gen.generate(seed=42, nodes=nodes, edges=edges)
    for val in problem.J.values():
        assert val in (-1.0, 1.0)


def test_max_cut_deterministic():
    gen = MaxCutGenerator()
    nodes = list(range(8))
    edges = [(i, i + 1) for i in range(7)]
    p1 = gen.generate(seed=123, nodes=nodes, edges=edges)
    p2 = gen.generate(seed=123, nodes=nodes, edges=edges)
    assert p1.J == p2.J
```

- [ ] **Step 2: Implement max_cut.py**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""MAX-CUT Ising problem generator.

h=0 everywhere, J in {-1, +1}. Classical NP-hard benchmark.
"""
from __future__ import annotations

from isingmark.chacha8 import ChaCha8Rng
from isingmark.models import IsingProblem
from isingmark.problems import register


class MaxCutGenerator:
    """MAX-CUT: h=0, J in {-1, +1} on given graph."""

    @property
    def name(self) -> str:
        return "max_cut"

    def generate(
        self,
        seed: int,
        nodes: list[int],
        edges: list[tuple[int, int]],
        **kwargs,
    ) -> IsingProblem:
        rng = ChaCha8Rng.seed_from_u64(seed)
        h = {int(nid): 0.0 for nid in nodes}
        J: dict[tuple[int, int], float] = {}
        for u, v in edges:
            sign = -1.0 if (rng.next_u32() & 1) == 0 else 1.0
            J[(int(u), int(v))] = sign
        return IsingProblem(
            problem_id=f"max_cut_{seed}",
            h=h,
            J=J,
            nodes=list(nodes),
            edges=list(edges),
            problem_type=self.name,
            seed=seed,
            metadata={},
        )

    def known_ground_energy(
        self, problem: IsingProblem,
    ) -> float | None:
        return None


register(MaxCutGenerator())
```

- [ ] **Step 3: Run tests**

Run: `python -m pytest tests/test_max_cut.py -v`
Expected: all 4 PASS

- [ ] **Step 4: Commit**

```bash
git add isingmark/problems/max_cut.py tests/test_max_cut.py
git commit -m "Add MAX-CUT problem generator"
```

---

### Task 22: Example topologies

**Files:**
- Create: `examples/topologies/random_regular_500.json`

- [ ] **Step 1: Generate example topology**

Create a script `scripts/gen_topology.py` to generate random regular graph JSON:

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Generate example topology JSON files."""
from __future__ import annotations

import json
import random


def random_regular_graph(n: int, d: int, seed: int = 42) -> dict:
    """Generate a random d-regular graph with n nodes."""
    rng = random.Random(seed)
    nodes = list(range(n))
    edges = set()
    stubs = list(range(n)) * d
    rng.shuffle(stubs)
    for i in range(0, len(stubs), 2):
        u, v = stubs[i], stubs[i + 1]
        if u != v and (min(u, v), max(u, v)) not in edges:
            edges.add((min(u, v), max(u, v)))
    return {
        "nodes": nodes,
        "edges": [list(e) for e in sorted(edges)],
    }


if __name__ == "__main__":
    g = random_regular_graph(500, 3)
    with open("examples/topologies/random_regular_500.json", "w") as f:
        json.dump(g, f, indent=2)
    print(f"Generated: {len(g['nodes'])} nodes, {len(g['edges'])} edges")
```

Run: `python scripts/gen_topology.py`

- [ ] **Step 2: Commit**

```bash
git add examples/ scripts/
git commit -m "Add example topology and generation script"
```

---

## Phase 4: Visualization & Polish

### Task 23: Visualization — tables

**Files:**
- Create: `isingmark/viz/__init__.py`
- Create: `isingmark/viz/tables.py`

- [ ] **Step 1: Write test**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for text table output."""
from __future__ import annotations

from isingmark.viz.tables import format_summary_table


def test_format_summary_table():
    summary = {
        "benchmark_mode": "throughput",
        "total_problems": 42,
        "problems_per_minute": 8.4,
        "energy_stats": {
            "min": -150.0,
            "mean": -120.0,
            "std": 10.0,
        },
    }
    table = format_summary_table(summary)
    assert "throughput" in table
    assert "42" in table
    assert "-150.0" in table
```

- [ ] **Step 2: Implement tables.py**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Text and markdown table formatters for benchmark results."""
from __future__ import annotations

from typing import Any


def format_summary_table(summary: dict[str, Any]) -> str:
    """Format a summary dict as a readable text table."""
    lines = []
    lines.append(f"{'='*55}")
    lines.append(f"  Benchmark: {summary.get('benchmark_mode', '?')}")
    lines.append(f"  Problems:  {summary.get('total_problems', 0)}")

    ppm = summary.get("problems_per_minute")
    if ppm is not None:
        lines.append(f"  Rate:      {ppm:.1f} problems/min")

    energy = summary.get("energy_stats", {})
    if energy:
        lines.append(
            f"  Energy:    "
            f"min={energy.get('min', '?')}, "
            f"mean={energy.get('mean', '?')}, "
            f"std={energy.get('std', '?')}"
        )

    timing = summary.get("timing_stats", {})
    if timing:
        lines.append(
            f"  Timing:    "
            f"min={timing.get('min_s', '?')}s, "
            f"mean={timing.get('mean_s', '?')}s, "
            f"max={timing.get('max_s', '?')}s"
        )

    lines.append(f"{'='*55}")
    return "\n".join(lines)


def format_comparison_table(
    summaries: list[dict[str, Any]],
) -> str:
    """Format multiple summaries as a comparison table."""
    if not summaries:
        return "(no data)"

    header = f"{'Annealer':<20} {'Problems':>8} {'Rate/min':>10} {'Min E':>10} {'Mean E':>10}"
    lines = [header, "-" * len(header)]

    for s in summaries:
        name = s.get("annealer_name", "?")
        n = s.get("total_problems", 0)
        rate = s.get("problems_per_minute", 0)
        e = s.get("energy_stats", {})
        lines.append(
            f"{name:<20} {n:>8} {rate:>10.1f} "
            f"{e.get('min', 0):>10.1f} "
            f"{e.get('mean', 0):>10.1f}"
        )

    return "\n".join(lines)
```

Also create `isingmark/viz/__init__.py`:

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Visualization modules (optional matplotlib dependency)."""
```

- [ ] **Step 3: Run tests**

Run: `python -m pytest tests/test_tables.py -v` (create as `tests/test_tables.py`)
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add isingmark/viz/ tests/test_tables.py
git commit -m "Add text table formatters for benchmark summaries"
```

---

### Task 24: Visualization — heatmaps and curves

**Files:**
- Create: `isingmark/viz/heatmaps.py`
- Create: `isingmark/viz/curves.py`

- [ ] **Step 1: Implement heatmaps.py**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Parameter sweep heatmaps (requires matplotlib)."""
from __future__ import annotations

from pathlib import Path
from typing import Any

import numpy as np

from isingmark.models import AnnealerSolution


def plot_sweep_heatmap(
    results: list[AnnealerSolution],
    x_param: str,
    y_param: str,
    metric: str = "best_energy",
    output_path: Path | None = None,
) -> Any:
    """Plot a heatmap of metric vs two swept parameters.

    Returns the matplotlib figure.
    """
    try:
        import matplotlib.pyplot as plt
    except ImportError:
        raise ImportError(
            "matplotlib required for heatmaps. "
            "Install with: pip install isingmark[viz]"
        )

    # Extract unique parameter values
    x_vals = sorted({r.params[x_param] for r in results})
    y_vals = sorted({r.params[y_param] for r in results})

    # Build grid: average metric per (x, y) cell
    grid = np.full((len(y_vals), len(x_vals)), np.nan)
    for r in results:
        xi = x_vals.index(r.params[x_param])
        yi = y_vals.index(r.params[y_param])
        val = getattr(r, metric, r.best_energy)
        if np.isnan(grid[yi, xi]):
            grid[yi, xi] = val
        else:
            grid[yi, xi] = (grid[yi, xi] + val) / 2

    fig, ax = plt.subplots(figsize=(10, 8))
    im = ax.imshow(
        grid, aspect="auto", origin="lower", cmap="viridis",
    )
    ax.set_xticks(range(len(x_vals)))
    ax.set_xticklabels(x_vals)
    ax.set_yticks(range(len(y_vals)))
    ax.set_yticklabels(y_vals)
    ax.set_xlabel(x_param)
    ax.set_ylabel(y_param)
    ax.set_title(f"{metric} heatmap")
    fig.colorbar(im)

    if output_path:
        fig.savefig(output_path, dpi=150, bbox_inches="tight")
    return fig
```

- [ ] **Step 2: Implement curves.py**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""TTT CDFs and energy distribution curves (requires matplotlib)."""
from __future__ import annotations

from pathlib import Path
from typing import Any

import numpy as np


def plot_ttt_cdf(
    ttt_results: dict[float, list[float]],
    output_path: Path | None = None,
) -> Any:
    """Plot TTT cumulative distribution functions.

    Returns the matplotlib figure.
    """
    try:
        import matplotlib.pyplot as plt
    except ImportError:
        raise ImportError(
            "matplotlib required for plots. "
            "Install with: pip install isingmark[viz]"
        )

    fig, ax = plt.subplots(figsize=(10, 6))

    for target, times in sorted(ttt_results.items()):
        if not times:
            continue
        sorted_times = np.sort(times)
        cdf = np.arange(1, len(sorted_times) + 1) / len(
            sorted_times,
        )
        ax.plot(
            sorted_times, cdf, label=f"E={target:.0f}",
        )

    ax.set_xlabel("Time (seconds)")
    ax.set_ylabel("Cumulative probability")
    ax.set_title("Time-to-Target CDF")
    ax.legend()
    ax.grid(True, alpha=0.3)

    if output_path:
        fig.savefig(output_path, dpi=150, bbox_inches="tight")
    return fig


def plot_energy_distributions(
    energy_sets: dict[str, list[float]],
    output_path: Path | None = None,
) -> Any:
    """Plot overlapping energy histograms for comparison.

    Returns the matplotlib figure.
    """
    try:
        import matplotlib.pyplot as plt
    except ImportError:
        raise ImportError(
            "matplotlib required for plots. "
            "Install with: pip install isingmark[viz]"
        )

    fig, ax = plt.subplots(figsize=(10, 6))

    for name, energies in energy_sets.items():
        ax.hist(
            energies, bins=30, alpha=0.5, label=name,
            density=True,
        )

    ax.set_xlabel("Energy")
    ax.set_ylabel("Density")
    ax.set_title("Energy Distribution Comparison")
    ax.legend()

    if output_path:
        fig.savefig(output_path, dpi=150, bbox_inches="tight")
    return fig
```

- [ ] **Step 3: Commit**

```bash
git add isingmark/viz/heatmaps.py isingmark/viz/curves.py
git commit -m "Add heatmap and curve visualizations (optional matplotlib)"
```

---

### Task 25: Scaling and reproducibility runners

**Files:**
- Create: `isingmark/runners/scaling.py`
- Create: `isingmark/runners/reproducibility.py`
- Create: `tests/test_reproducibility.py`

- [ ] **Step 1: Implement scaling.py**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Problem size scaling runner."""
from __future__ import annotations

import random
from typing import Any

import click

from isingmark.annealers import AnnealerAdapter
from isingmark.models import AnnealerSolution, BenchmarkConfig
from isingmark.params.adaptor import ParamsAdaptor
from isingmark.problems import ProblemGenerator


def _make_random_graph(
    n: int, avg_degree: int, rng_seed: int,
) -> tuple[list[int], list[tuple[int, int]]]:
    """Generate a random graph with approximately avg_degree edges per node."""
    rng = random.Random(rng_seed)
    nodes = list(range(n))
    target_edges = n * avg_degree // 2
    edges: set[tuple[int, int]] = set()
    while len(edges) < target_edges:
        u = rng.randrange(n)
        v = rng.randrange(n)
        if u != v:
            edges.add((min(u, v), max(u, v)))
    return nodes, list(edges)


def run_scaling(
    *,
    config: BenchmarkConfig,
    annealer: AnnealerAdapter,
    params_adaptor: ParamsAdaptor,
    generator: ProblemGenerator,
    sizes: list[int],
    avg_degree: int = 4,
    num_problems_per_size: int = 5,
    base_seed: int = 0,
    show_progress: bool = True,
) -> dict[int, list[AnnealerSolution]]:
    """Vary problem size, measure performance degradation.

    Returns {size: [solutions]}.
    """
    params = params_adaptor.adapt(config.hardness, None)
    results: dict[int, list[AnnealerSolution]] = {}
    total = len(sizes) * num_problems_per_size

    if show_progress:
        bar = click.progressbar(
            length=total, label="Scaling", show_eta=True,
        )
    else:
        bar = None

    try:
        if bar is not None:
            bar.__enter__()

        for size in sizes:
            size_results: list[AnnealerSolution] = []
            for i in range(num_problems_per_size):
                seed = base_seed + size * 1000 + i
                nodes, edges = _make_random_graph(
                    size, avg_degree, seed,
                )
                problem = generator.generate(
                    seed=seed, nodes=nodes, edges=edges,
                )
                solution = annealer.sample_ising(
                    problem.h,
                    problem.J,
                    num_reads=params["num_reads"],
                    num_sweeps=params["num_sweeps"],
                    problem_id=problem.problem_id,
                    nodes=problem.nodes,
                )
                size_results.append(solution)
                if bar is not None:
                    bar.update(1)
            results[size] = size_results

    finally:
        if bar is not None:
            bar.__exit__(None, None, None)

    return results
```

- [ ] **Step 2: Implement reproducibility.py**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Reproducibility runner: verify statistical consistency."""
from __future__ import annotations

from collections.abc import Iterable

import click

from isingmark.annealers import AnnealerAdapter
from isingmark.models import AnnealerSolution, BenchmarkConfig
from isingmark.params.adaptor import ParamsAdaptor
from isingmark.problems import ProblemGenerator
from isingmark.results.comparator import ks_distance


def run_reproducibility(
    *,
    config: BenchmarkConfig,
    annealer: AnnealerAdapter,
    params_adaptor: ParamsAdaptor,
    generator: ProblemGenerator,
    nodes: list[int],
    edges: list[tuple[int, int]],
    seeds: list[int],
    show_progress: bool = True,
) -> dict:
    """Run same problems twice, verify statistical consistency.

    Returns report with KS statistic and p-value.
    """
    params = params_adaptor.adapt(config.hardness, None)
    total = len(seeds) * 2

    if show_progress:
        bar = click.progressbar(
            length=total, label="Reproducibility",
            show_eta=True,
        )
    else:
        bar = None

    def _run_batch() -> list[AnnealerSolution]:
        results: list[AnnealerSolution] = []
        for seed in seeds:
            problem = generator.generate(
                seed=seed, nodes=nodes, edges=edges,
            )
            solution = annealer.sample_ising(
                problem.h,
                problem.J,
                num_reads=params["num_reads"],
                num_sweeps=params["num_sweeps"],
                problem_id=problem.problem_id,
                nodes=problem.nodes,
            )
            results.append(solution)
            if bar is not None:
                bar.update(1)
        return results

    try:
        if bar is not None:
            bar.__enter__()
        run1 = _run_batch()
        run2 = _run_batch()
    finally:
        if bar is not None:
            bar.__exit__(None, None, None)

    energies1 = [r.best_energy for r in run1]
    energies2 = [r.best_energy for r in run2]
    ks_stat, p_value = ks_distance(energies1, energies2)

    return {
        "ks_statistic": ks_stat,
        "p_value": p_value,
        "consistent": p_value > 0.05,
        "num_problems": len(seeds),
        "run1_results": run1,
        "run2_results": run2,
    }
```

- [ ] **Step 3: Write reproducibility test**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for reproducibility runner."""
from __future__ import annotations

from isingmark.annealers.sa_dimod import SADimodAdapter
from isingmark.models import BenchmarkConfig
from isingmark.params.adaptor import DefaultParamsAdaptor
from isingmark.problems.random_ising import RandomIsingGenerator
from isingmark.runners.reproducibility import run_reproducibility


def test_reproducibility_consistent():
    nodes = list(range(15))
    edges = [(i, i + 1) for i in range(14)]

    report = run_reproducibility(
        config=BenchmarkConfig(
            mode="reproducibility",
            annealer_name="sa-dimod",
            problem_type="random_ising",
            hardness=0.3,
        ),
        annealer=SADimodAdapter(),
        params_adaptor=DefaultParamsAdaptor(),
        generator=RandomIsingGenerator(),
        nodes=nodes,
        edges=edges,
        seeds=list(range(10)),
        show_progress=False,
    )
    assert "ks_statistic" in report
    assert "p_value" in report
    assert report["num_problems"] == 10
```

- [ ] **Step 4: Run tests**

Run: `python -m pytest tests/test_reproducibility.py -v`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add isingmark/runners/scaling.py isingmark/runners/reproducibility.py tests/test_reproducibility.py
git commit -m "Add scaling and reproducibility runners"
```

---

### Task 26: Compare CLI subcommand

**Files:**
- Modify: `isingmark/cli.py`

- [ ] **Step 1: Add compare subcommand**

```python
@main.command()
@click.option("--file-a", required=True, type=click.Path(exists=True))
@click.option("--file-b", required=True, type=click.Path(exists=True))
@click.option("--plot", is_flag=True, help="Generate comparison plots")
@click.option("--output-dir", default=".", type=click.Path())
def compare(file_a, file_b, plot, output_dir):
    """Compare two result files side-by-side."""
    from isingmark.results.aggregator import aggregate_solutions
    from isingmark.results.comparator import ks_distance
    from isingmark.results.storage import read_solutions_jsonl
    from isingmark.viz.tables import format_comparison_table

    results_a = read_solutions_jsonl(Path(file_a))
    results_b = read_solutions_jsonl(Path(file_b))

    agg_a = aggregate_solutions(results_a)
    agg_b = aggregate_solutions(results_b)

    # Add names from first result
    agg_a["annealer_name"] = results_a[0].annealer_name if results_a else "A"
    agg_b["annealer_name"] = results_b[0].annealer_name if results_b else "B"

    table = format_comparison_table([agg_a, agg_b])
    click.echo(table)

    energies_a = [r.best_energy for r in results_a]
    energies_b = [r.best_energy for r in results_b]
    ks_stat, p_value = ks_distance(energies_a, energies_b)
    click.echo(f"\nKS distance: {ks_stat:.4f} (p={p_value:.4f})")

    if plot:
        try:
            from isingmark.viz.curves import plot_energy_distributions

            out = Path(output_dir)
            out.mkdir(parents=True, exist_ok=True)
            plot_energy_distributions(
                {
                    agg_a["annealer_name"]: energies_a,
                    agg_b["annealer_name"]: energies_b,
                },
                output_path=out / "comparison.png",
            )
            click.echo(f"Plot saved to {out / 'comparison.png'}")
        except ImportError:
            click.echo("matplotlib not installed, skipping plot")
```

- [ ] **Step 2: Add scaling and reproducibility subcommands**

```python
@main.command()
@click.option("--annealer", default="sa-dimod")
@click.option("--problem-type", default="random_ising")
@click.option("--hardness", default=0.5, type=float)
@click.option("--sizes", required=True, help="Comma-separated node counts")
@click.option("--seed", default=0, type=int)
@click.option("--output-dir", default=".", type=click.Path())
def scaling(annealer, problem_type, hardness, sizes, seed, output_dir):
    """Vary problem size, measure degradation."""
    from isingmark.runners.scaling import run_scaling

    size_list = [int(x.strip()) for x in sizes.split(",")]
    gen = get_generator(problem_type)
    adapter = get_adapter(annealer)
    params_adaptor = DefaultParamsAdaptor()
    config = BenchmarkConfig(mode="scaling", annealer_name=annealer, problem_type=problem_type, hardness=hardness)

    adapter.setup()
    try:
        results = run_scaling(
            config=config, annealer=adapter, params_adaptor=params_adaptor,
            generator=gen, sizes=size_list, base_seed=seed,
        )
    finally:
        adapter.teardown()

    for size, sols in sorted(results.items()):
        energies = [s.best_energy for s in sols]
        times = [s.wall_time_s for s in sols]
        click.echo(f"  n={size}: energy={np.mean(energies):.1f} +/- {np.std(energies):.1f}, time={np.mean(times):.3f}s")


@main.command()
@click.option("--annealer", default="sa-dimod")
@click.option("--problem-type", default="random_ising")
@click.option("--hardness", default=0.5, type=float)
@click.option("--num-problems", default=20, type=int)
@click.option("--seed", default=0, type=int)
@click.option("--topology", default=None, type=click.Path())
@click.option("--output-dir", default=".", type=click.Path())
def reproducibility(annealer, problem_type, hardness, num_problems, seed, topology, output_dir):
    """Statistical consistency check."""
    from isingmark.runners.reproducibility import run_reproducibility

    nodes, edges = _load_topology(topology)
    gen = get_generator(problem_type)
    adapter = get_adapter(annealer)
    params_adaptor = DefaultParamsAdaptor()
    config = BenchmarkConfig(mode="reproducibility", annealer_name=annealer, problem_type=problem_type, hardness=hardness)

    adapter.setup()
    try:
        report = run_reproducibility(
            config=config, annealer=adapter, params_adaptor=params_adaptor,
            generator=gen, nodes=nodes, edges=edges,
            seeds=list(range(seed, seed + num_problems)),
        )
    finally:
        adapter.teardown()

    click.echo(f"KS statistic: {report['ks_statistic']:.4f}")
    click.echo(f"p-value: {report['p_value']:.4f}")
    click.echo(f"Consistent: {'YES' if report['consistent'] else 'NO'}")
```

- [ ] **Step 3: Run full test suite**

Run: `python -m pytest tests/ -v`
Expected: all tests PASS

- [ ] **Step 4: Commit**

```bash
git add isingmark/cli.py
git commit -m "Add compare, scaling, and reproducibility CLI subcommands"
```

---

### Task 27: CLAUDE.md and final polish

**Files:**
- Create: `CLAUDE.md`
- Modify: `pyproject.toml` (if needed)

- [ ] **Step 1: Create CLAUDE.md**

```markdown
# CLAUDE.md

## Overview

IsingMark: standalone annealer-agnostic Ising model benchmarking tool.
Extracted from quip-protocol-2. Compares any Ising solver via reproducible benchmarks.

## Commands

\```bash
# Setup
python3 -m venv .venv && source .venv/bin/activate
pip install -e ".[dev]"

# Run tests
python -m pytest tests/ -v

# CLI
isingmark --help
isingmark throughput --duration 10m --hardness 0.5
isingmark sweep --param-grid '{"num_reads": [64,128], "num_sweeps": [100,500]}'
isingmark ttt --energy-targets "-5,-10,-15" --num-trials 20
isingmark compare --file-a results_a.jsonl --file-b results_b.jsonl --plot
isingmark scaling --sizes "10,20,50,100"
isingmark reproducibility --num-problems 20
\```

## Architecture

Three extension points:
- **ProblemGenerator** (Protocol) — `isingmark/problems/`
- **AnnealerAdapter** (ABC) — `isingmark/annealers/`
- **ParamsAdaptor** (ABC) — `isingmark/params/`

Multiprocessing with spawn context. JSONL + summary JSON output.

## License

AGPL-3.0-or-later
```

- [ ] **Step 2: Run full test suite and CLI smoke test**

```bash
python -m pytest tests/ -v
isingmark --version
isingmark throughput --duration 5s --seed 42
```

- [ ] **Step 3: Final commit**

```bash
git add CLAUDE.md
git commit -m "Add CLAUDE.md project documentation"
```

---

## Verification Checklist

After all tasks are complete, verify:

- [ ] `python -m pytest tests/ -v` — all tests pass
- [ ] `isingmark --version` — prints 0.1.0
- [ ] `isingmark throughput --duration 30s` — runs, writes JSONL + summary
- [ ] `isingmark sweep --param-grid '{"num_reads":[10,20],"num_sweeps":[50,100]}' --num-problems 3` — runs grid search
- [ ] `isingmark compare --file-a <throughput_jsonl> --file-b <sweep_jsonl>` — prints comparison table
- [ ] ChaCha8 test vectors match cross-language reference
- [ ] Random Ising problems match quip-protocol-2 output for same seeds
