# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for parameter sweep runner."""
from __future__ import annotations

from isingmark.annealers import get_adapter
from isingmark.models import BenchmarkConfig
from isingmark.problems import get_generator
from isingmark.runners.sweep import run_sweep


NODES = list(range(6))
EDGES = [(i, i + 1) for i in range(5)]


def _make_problems(n: int = 3) -> list:
    gen = get_generator("random_ising")
    return [gen.generate(seed=i, nodes=NODES, edges=EDGES) for i in range(n)]


def test_sweep_small_grid() -> None:
    """2 num_reads x 2 num_sweeps x 3 problems = 12 results."""
    config = BenchmarkConfig(
        mode="sweep",
        annealer_name="sa-dimod",
        problem_type="random_ising",
    )
    annealer = get_adapter("sa-dimod")
    problems = _make_problems(3)
    param_grid = {
        "num_reads": [10, 20],
        "num_sweeps": [50, 100],
    }

    results = run_sweep(config, annealer, param_grid, problems, show_progress=False)

    assert len(results) == 12, f"Expected 12 results, got {len(results)}"
    for r in results:
        assert r.annealer_name == "sa-dimod"
        assert r.best_energy <= r.mean_energy + 1e-9


def test_sweep_empty_grid() -> None:
    """Empty param_grid produces one combo (the empty combo) × n problems."""
    config = BenchmarkConfig(
        mode="sweep",
        annealer_name="sa-dimod",
        problem_type="random_ising",
    )
    annealer = get_adapter("sa-dimod")
    problems = _make_problems(2)

    # itertools.product() over zero iterables yields exactly one empty combo,
    # so we get one run per problem with default annealer parameters.
    results = run_sweep(config, annealer, {}, problems, show_progress=False)
    assert len(results) == 2


def test_sweep_single_combo() -> None:
    """Single combo × single problem → 1 result."""
    config = BenchmarkConfig(
        mode="sweep",
        annealer_name="sa-dimod",
        problem_type="random_ising",
    )
    annealer = get_adapter("sa-dimod")
    problems = _make_problems(1)
    param_grid = {"num_reads": [5], "num_sweeps": [10]}

    results = run_sweep(config, annealer, param_grid, problems, show_progress=False)
    assert len(results) == 1


def test_sweep_result_order() -> None:
    """Results are ordered: (combo_0, p0), (combo_0, p1), (combo_1, p0), ..."""
    config = BenchmarkConfig(
        mode="sweep",
        annealer_name="sa-dimod",
        problem_type="random_ising",
    )
    annealer = get_adapter("sa-dimod")
    problems = _make_problems(2)
    param_grid = {"num_reads": [5, 10]}

    results = run_sweep(config, annealer, param_grid, problems, show_progress=False)
    assert len(results) == 4
    # Verify problem_ids correspond to expected order
    assert results[0].problem_id == problems[0].problem_id
    assert results[1].problem_id == problems[1].problem_id
    assert results[2].problem_id == problems[0].problem_id
    assert results[3].problem_id == problems[1].problem_id
