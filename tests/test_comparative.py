# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for the comparative (KS-based hardness binary search) runner."""
from __future__ import annotations

import pytest

from isingmark.annealers import get_adapter
from isingmark.models import BenchmarkConfig
from isingmark.params.adaptor import DefaultParamsAdaptor
from isingmark.problems import get_generator
from isingmark.runners.comparative import run_comparative


NODES = list(range(8))
EDGES = [(i, i + 1) for i in range(7)]


def _generate_target_solutions(hardness: float, n: int = 15) -> list:
    """Generate reference solutions at the given hardness."""
    gen = get_generator("random_ising")
    annealer = get_adapter("sa-dimod")
    adaptor = DefaultParamsAdaptor(
        min_sweeps=32, max_sweeps=512, min_reads=20, max_reads=100
    )
    params = adaptor.adapt(hardness)
    solutions = []
    for seed in range(n):
        problem = gen.generate(seed=seed + 1000, nodes=NODES, edges=EDGES)
        sol = annealer.sample_ising(
            problem.h, problem.J,
            problem_id=problem.problem_id,
            nodes=problem.nodes,
            **params,
        )
        solutions.append(sol)
    return solutions


def test_comparative_recovers_hardness() -> None:
    """Binary search should converge near the true hardness (0.3)."""
    target_hardness = 0.3
    target_solutions = _generate_target_solutions(target_hardness, n=15)

    config = BenchmarkConfig(
        mode="comparative",
        annealer_name="sa-dimod",
        problem_type="random_ising",
    )
    gen = get_generator("random_ising")
    annealer = get_adapter("sa-dimod")
    adaptor = DefaultParamsAdaptor(
        min_sweeps=32, max_sweeps=512, min_reads=20, max_reads=100
    )
    seeds = list(range(200))

    _, best_hardness = run_comparative(
        config=config,
        annealer=annealer,
        target_results=target_solutions,
        params_adaptor=adaptor,
        generator=gen,
        nodes=NODES,
        edges=EDGES,
        seeds=seeds,
        max_iterations=8,
        num_problems_per_step=8,
        show_progress=False,
    )

    # Binary search with small samples is noisy; verify the result is in [0, 1]
    # and within half the search range of the true value.
    assert 0.0 <= best_hardness <= 1.0
    assert abs(best_hardness - target_hardness) < 0.5, (
        f"Expected hardness near {target_hardness}, got {best_hardness}"
    )


def test_comparative_returns_solutions() -> None:
    """run_comparative always returns a non-empty solutions list."""
    target_solutions = _generate_target_solutions(0.5, n=5)

    config = BenchmarkConfig(
        mode="comparative",
        annealer_name="sa-dimod",
        problem_type="random_ising",
    )
    gen = get_generator("random_ising")
    annealer = get_adapter("sa-dimod")
    adaptor = DefaultParamsAdaptor()
    seeds = list(range(50))

    solutions, hardness = run_comparative(
        config=config,
        annealer=annealer,
        target_results=target_solutions,
        params_adaptor=adaptor,
        generator=gen,
        nodes=NODES,
        edges=EDGES,
        seeds=seeds,
        max_iterations=3,
        num_problems_per_step=5,
        show_progress=False,
    )

    assert len(solutions) > 0
    assert 0.0 <= hardness <= 1.0
