# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for results aggregator."""
from __future__ import annotations

import time

import pytest

from isingmark.models import AnnealerSolution
from isingmark.results.aggregator import aggregate_solutions


def _make_solution(
    best_energy: float,
    wall_time_s: float,
    diversity: float = 0.5,
) -> AnnealerSolution:
    return AnnealerSolution(
        problem_id="test",
        annealer_name="sa-dimod",
        params={},
        wall_time_s=wall_time_s,
        best_energy=best_energy,
        mean_energy=best_energy,
        std_energy=0.0,
        num_samples=10,
        energies=[best_energy] * 10,
        diversity=diversity,
        num_below_target=0,
        satisfaction_rate=1.0,
        hardware_timing={},
        timestamp=time.time(),
    )


def test_aggregate_empty() -> None:
    """Empty input returns only total_problems=0."""
    result = aggregate_solutions([])
    assert result == {"total_problems": 0}


def test_aggregate_energy_stats() -> None:
    """Check energy stats for three solutions with known energies."""
    solutions = [
        _make_solution(best_energy=-3.0, wall_time_s=0.1),
        _make_solution(best_energy=-1.0, wall_time_s=0.2),
        _make_solution(best_energy=-2.0, wall_time_s=0.3),
    ]
    result = aggregate_solutions(solutions)

    assert result["total_problems"] == 3
    stats = result["energy_stats"]
    assert stats["min"] == pytest.approx(-3.0)
    assert stats["max"] == pytest.approx(-1.0)
    assert stats["mean"] == pytest.approx(-2.0)
    assert stats["p50"] == pytest.approx(-2.0)
    assert "std" in stats
    assert "p25" in stats
    assert "p75" in stats


def test_aggregate_timing_stats() -> None:
    """Check timing stats for two solutions."""
    solutions = [
        _make_solution(best_energy=-1.0, wall_time_s=0.1),
        _make_solution(best_energy=-2.0, wall_time_s=0.3),
    ]
    result = aggregate_solutions(solutions)

    timing = result["timing_stats"]
    assert timing["min_s"] == pytest.approx(0.1)
    assert timing["max_s"] == pytest.approx(0.3)
    assert timing["mean_s"] == pytest.approx(0.2)
    assert "std_s" in timing


def test_aggregate_diversity_stats() -> None:
    """Check diversity stats are present and correctly computed."""
    solutions = [
        _make_solution(best_energy=-1.0, wall_time_s=0.1, diversity=0.2),
        _make_solution(best_energy=-2.0, wall_time_s=0.1, diversity=0.8),
    ]
    result = aggregate_solutions(solutions)

    div = result["diversity_stats"]
    assert div["min"] == pytest.approx(0.2)
    assert div["max"] == pytest.approx(0.8)
    assert div["mean"] == pytest.approx(0.5)


def test_aggregate_single_solution() -> None:
    """Single solution: std should be 0."""
    solutions = [_make_solution(best_energy=-5.0, wall_time_s=1.0)]
    result = aggregate_solutions(solutions)
    assert result["total_problems"] == 1
    assert result["energy_stats"]["std"] == pytest.approx(0.0)
