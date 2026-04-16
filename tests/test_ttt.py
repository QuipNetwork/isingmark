# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for the time-to-target (TTT) runner."""
from __future__ import annotations

import pytest

from isingmark.annealers import get_adapter
from isingmark.models import BenchmarkConfig
from isingmark.params.adaptor import DefaultParamsAdaptor
from isingmark.problems import get_generator
from isingmark.runners.ttt import run_ttt


# 15-node chain topology
NODES = list(range(15))
EDGES = [(i, i + 1) for i in range(14)]


def test_ttt_achievable_target() -> None:
    """An easy target (-1.0 for a 15-node chain) should be reached in most trials."""
    config = BenchmarkConfig(
        mode="ttt",
        annealer_name="sa-dimod",
        problem_type="random_ising",
        hardness=0.5,
    )
    gen = get_generator("random_ising")
    annealer = get_adapter("sa-dimod")
    adaptor = DefaultParamsAdaptor(
        min_sweeps=200, max_sweeps=1000, min_reads=50, max_reads=200
    )
    seeds = list(range(15))

    results = run_ttt(
        config=config,
        annealer=annealer,
        params_adaptor=adaptor,
        generator=gen,
        nodes=NODES,
        edges=EDGES,
        energy_targets=[-1.0],
        num_trials=15,
        max_time_per_target=30.0,
        seeds=seeds,
        show_progress=False,
    )

    assert -1.0 in results
    times = results[-1.0]
    # At least some trials should succeed for such an easy target
    assert len(times) > 0, "Expected at least one successful trial for target -1.0"
    for t in times:
        assert t >= 0.0


def test_ttt_impossible_target() -> None:
    """A target well below any possible energy should yield no successes."""
    config = BenchmarkConfig(
        mode="ttt",
        annealer_name="sa-dimod",
        problem_type="random_ising",
        hardness=0.1,
    )
    gen = get_generator("random_ising")
    annealer = get_adapter("sa-dimod")
    adaptor = DefaultParamsAdaptor(
        min_sweeps=10, max_sweeps=20, min_reads=5, max_reads=10
    )
    seeds = list(range(3))

    results = run_ttt(
        config=config,
        annealer=annealer,
        params_adaptor=adaptor,
        generator=gen,
        nodes=NODES,
        edges=EDGES,
        energy_targets=[-9999.0],
        num_trials=3,
        max_time_per_target=0.05,  # very short budget
        seeds=seeds,
        show_progress=False,
    )

    assert -9999.0 in results
    # With a budget of 50 ms and an impossible target, no successes expected
    assert results[-9999.0] == []


def test_ttt_multiple_targets() -> None:
    """Results dict contains an entry for every requested target."""
    config = BenchmarkConfig(
        mode="ttt",
        annealer_name="sa-dimod",
        problem_type="random_ising",
        hardness=0.3,
    )
    gen = get_generator("random_ising")
    annealer = get_adapter("sa-dimod")
    adaptor = DefaultParamsAdaptor()
    targets = [-5.0, -10.0, -50.0]

    results = run_ttt(
        config=config,
        annealer=annealer,
        params_adaptor=adaptor,
        generator=gen,
        nodes=NODES,
        edges=EDGES,
        energy_targets=targets,
        num_trials=2,
        max_time_per_target=0.1,
        seeds=list(range(5)),
        show_progress=False,
    )

    assert set(results.keys()) == set(targets)
    for key in targets:
        assert isinstance(results[key], list)
