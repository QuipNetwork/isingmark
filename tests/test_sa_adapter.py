# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for the SA-dimod annealer adapter."""
from __future__ import annotations

import pytest

from isingmark.annealers import get_adapter, list_adapters
from isingmark.models import AnnealerSolution


def _chain_problem(n: int) -> tuple[dict[int, float], dict[tuple[int, int], float]]:
    """Build an n-node ferromagnetic chain with zero external field."""
    h = {i: 0.0 for i in range(n)}
    J = {(i, i + 1): -1.0 for i in range(n - 1)}
    return h, J


def test_adapter_registered() -> None:
    adapter = get_adapter("sa-dimod")
    assert adapter is not None
    assert adapter.name == "sa-dimod"
    assert "sa-dimod" in list_adapters()


def test_sample_ising_returns_solution() -> None:
    adapter = get_adapter("sa-dimod")
    h, J = _chain_problem(10)
    sol = adapter.sample_ising(h, J, num_reads=20, num_sweeps=200, seed=42, problem_id="chain10")

    assert isinstance(sol, AnnealerSolution)
    assert sol.problem_id == "chain10"
    assert sol.annealer_name == "sa-dimod"
    assert sol.num_samples == 20
    assert len(sol.energies) == 20
    assert sol.wall_time_s >= 0.0
    assert sol.best_energy <= sol.mean_energy
    assert 0.0 <= sol.diversity <= 1.0
    assert 0.0 <= sol.satisfaction_rate <= 1.0
    assert sol.params["num_reads"] == 20
    assert sol.params["num_sweeps"] == 200


def test_sample_produces_valid_energies() -> None:
    """On a 20-node ferromagnetic chain the best energy should be negative."""
    adapter = get_adapter("sa-dimod")
    h, J = _chain_problem(20)
    sol = adapter.sample_ising(h, J, num_reads=50, num_sweeps=500, seed=0)

    assert sol.best_energy < 0.0
    assert all(isinstance(e, float) for e in sol.energies)
    assert len(sol.energies) == 50
