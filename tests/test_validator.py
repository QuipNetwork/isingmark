# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for isingmark.validator.SolutionValidator."""

import pytest

from isingmark.models import AnnealerSolution, IsingProblem
from isingmark.validator import SolutionValidator


# ---------------------------------------------------------------------------
# compute_energy
# ---------------------------------------------------------------------------


def test_compute_energy_simple():
    # h={0:-1, 1:1}, J={(0,1):-1}, spins=[1,-1], nodes=[0,1]
    # E = h[0]*s[0] + h[1]*s[1] + J[(0,1)]*s[0]*s[1]
    #   = (-1)(1) + (1)(-1) + (-1)(1)(-1)
    #   = -1 - 1 + 1 = -1
    energy = SolutionValidator.compute_energy(
        spins=[1, -1],
        h={0: -1.0, 1: 1.0},
        J={(0, 1): -1.0},
        nodes=[0, 1],
    )
    assert energy == pytest.approx(-1.0)


def test_compute_energy_all_aligned():
    # Triangle: nodes=[0,1,2], h=0, J=-1 on all edges, all spins +1
    # E = J01*1*1 + J12*1*1 + J02*1*1 = -1-1-1 = -3
    energy = SolutionValidator.compute_energy(
        spins=[1, 1, 1],
        h={},
        J={(0, 1): -1.0, (1, 2): -1.0, (0, 2): -1.0},
        nodes=[0, 1, 2],
    )
    assert energy == pytest.approx(-3.0)


# ---------------------------------------------------------------------------
# validate_format
# ---------------------------------------------------------------------------


def test_validate_format_valid():
    errors = SolutionValidator.validate_format(spins=[1, -1, 1], num_nodes=3)
    assert errors == []


def test_validate_format_wrong_length():
    errors = SolutionValidator.validate_format(spins=[1, -1], num_nodes=3)
    assert len(errors) >= 1
    assert any("length" in e.lower() or "2" in e for e in errors)


def test_validate_format_invalid_values():
    errors = SolutionValidator.validate_format(spins=[1, 0, -1], num_nodes=3)
    assert len(errors) >= 1
    assert any("0" in e or "value" in e.lower() or "invalid" in e.lower() for e in errors)


# ---------------------------------------------------------------------------
# compute_diversity
# ---------------------------------------------------------------------------


def test_compute_diversity_identical():
    # All same solutions -> 0 Hamming distance
    solutions = [[1, 1, -1], [1, 1, -1], [1, 1, -1]]
    diversity = SolutionValidator.compute_diversity(solutions)
    assert diversity == pytest.approx(0.0)


def test_compute_diversity_opposite():
    # [1,1,1] vs [-1,-1,-1] -> spin-flip symmetric, min(3,0)/3 = 0.0
    solutions = [[1, 1, 1], [-1, -1, -1]]
    diversity = SolutionValidator.compute_diversity(solutions)
    assert diversity == pytest.approx(0.0)


def test_compute_diversity_different():
    # [1,1,-1] vs [1,-1,1]
    # Normal Hamming: positions 1,2 differ -> 2
    # Inverted b: [-1,1,-1] vs [1,1,-1] -> position 0 differs -> 1
    # min(2,1)/3 = 1/3
    solutions = [[1, 1, -1], [1, -1, 1]]
    diversity = SolutionValidator.compute_diversity(solutions)
    assert diversity == pytest.approx(1.0 / 3.0)


# ---------------------------------------------------------------------------
# coupling_satisfaction_rate
# ---------------------------------------------------------------------------


def test_coupling_satisfaction_rate():
    # J=-1 (ferromagnetic), same spins -> J*s_i*s_j = -1*1*1 = -1 < 0 -> satisfied
    spins = [1, 1, 1]
    J = {(0, 1): -1.0, (1, 2): -1.0, (0, 2): -1.0}
    nodes = [0, 1, 2]
    rate = SolutionValidator.coupling_satisfaction_rate(spins, J, nodes)
    assert rate == pytest.approx(1.0)


def test_coupling_satisfaction_frustrated():
    # J=+1 (antiferromagnetic), same spins -> J*s_i*s_j = 1*1*1 = 1 > 0 -> not satisfied
    spins = [1, 1]
    J = {(0, 1): 1.0}
    nodes = [0, 1]
    rate = SolutionValidator.coupling_satisfaction_rate(spins, J, nodes)
    assert rate == pytest.approx(0.0)


# ---------------------------------------------------------------------------
# validate_solution
# ---------------------------------------------------------------------------


def _make_problem() -> IsingProblem:
    return IsingProblem(
        problem_id="val-001",
        h={0: -1.0, 1: 1.0},
        J={(0, 1): -1.0},
        nodes=[0, 1],
        edges=[(0, 1)],
        problem_type="random",
        seed=0,
        metadata={},
    )


def _make_result(spins_list: list[list[int]], energies: list[float]) -> AnnealerSolution:
    return AnnealerSolution(
        problem_id="val-001",
        annealer_name="test",
        params={},
        wall_time_s=0.0,
        best_energy=min(energies),
        mean_energy=sum(energies) / len(energies),
        std_energy=0.0,
        num_samples=len(spins_list),
        energies=energies,
        diversity=0.0,
        num_below_target=0,
        satisfaction_rate=0.0,
        hardware_timing={},
        timestamp=0.0,
    )


def test_validate_solution_basic():
    problem = _make_problem()
    # best solution: spins=[1,-1] -> E = -1-1+1 = -1
    result = _make_result([[1, -1], [1, 1]], [-1.0, 0.0])
    report = SolutionValidator.validate_solution(
        problem=problem,
        result=result,
        spins_list=[[1, -1], [1, 1]],
    )
    assert report["valid"] is True
    assert report["errors"] == []
    assert report["best_energy"] == pytest.approx(-1.0)
    assert "diversity" in report
    assert "satisfaction_rate" in report


def test_validate_solution_with_target():
    problem = _make_problem()
    # [1,-1]: E = (-1)(1) + (1)(-1) + (-1)(1)(-1) = -1-1+1 = -1
    # [-1,1]: E = (-1)(-1) + (1)(1) + (-1)(-1)(1) = 1+1+1 = 3
    result = _make_result([[1, -1], [-1, 1]], [-1.0, 3.0])
    report = SolutionValidator.validate_solution(
        problem=problem,
        result=result,
        spins_list=[[1, -1], [-1, 1]],
        energy_target=0.0,
    )
    assert "num_below_target" in report
    # only energy -1.0 < 0.0, so exactly 1 solution is below target
    assert report["num_below_target"] == 1
