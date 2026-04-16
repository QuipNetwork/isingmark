# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for the planted solution generator."""
from __future__ import annotations

import pytest

import isingmark.problems  # ensure registration
from isingmark.problems import get_generator, list_generators
from isingmark.problems.planted_solution import PlantedSolutionGenerator


NODES = list(range(10))
EDGES = [(i, i + 1) for i in range(9)]


def test_planted_registered():
    """PlantedSolutionGenerator must be in the global registry."""
    assert "planted" in list_generators()


def test_planted_ground_energy_known_zero_frustration():
    """With zero frustration, the planted state energy must be negative.

    Every bias is set to penalise deviating from the planted spin, and every
    coupling is set to prefer alignment, so the planted state should achieve a
    strictly negative energy on any non-trivial graph.
    """
    gen = PlantedSolutionGenerator()
    problem = gen.generate(seed=42, nodes=NODES, edges=EDGES, frustration=0.0)
    energy = gen.known_ground_energy(problem)
    assert energy is not None
    # With h[i] = -spin[i] and J aligned: energy = Σ(-spin²) + Σ(-1) < 0
    assert energy < 0.0


def test_planted_known_ground_energy_matches_manual():
    """known_ground_energy must equal manually computed energy."""
    gen = PlantedSolutionGenerator()
    problem = gen.generate(seed=7, nodes=NODES, edges=EDGES, frustration=0.0)
    planted = problem.metadata["planted_spins"]

    manual = sum(problem.h[n] * planted[n] for n in problem.nodes)
    manual += sum(j * planted[u] * planted[v] for (u, v), j in problem.J.items())

    assert gen.known_ground_energy(problem) == pytest.approx(manual)


def test_planted_spins_in_metadata():
    """planted_spins in metadata must cover every node with ±1 values."""
    gen = PlantedSolutionGenerator()
    problem = gen.generate(seed=1, nodes=NODES, edges=EDGES)
    spins = problem.metadata["planted_spins"]
    assert set(spins.keys()) == set(NODES)
    assert all(v in (-1, 1) for v in spins.values())


def test_planted_deterministic():
    """Same seed must produce the same problem."""
    gen = PlantedSolutionGenerator()
    p1 = gen.generate(seed=99, nodes=NODES, edges=EDGES)
    p2 = gen.generate(seed=99, nodes=NODES, edges=EDGES)
    assert p1.h == p2.h
    assert p1.J == p2.J
    assert p1.metadata["planted_spins"] == p2.metadata["planted_spins"]


def test_planted_different_seeds_differ():
    """Different seeds should (almost certainly) give different problems."""
    gen = PlantedSolutionGenerator()
    p1 = gen.generate(seed=0, nodes=NODES, edges=EDGES)
    p2 = gen.generate(seed=1, nodes=NODES, edges=EDGES)
    assert p1.h != p2.h or p1.J != p2.J


def test_planted_frustration_invalid():
    """frustration outside [0, 1] must raise ValueError."""
    gen = PlantedSolutionGenerator()
    with pytest.raises(ValueError, match="frustration"):
        gen.generate(seed=0, nodes=NODES, edges=EDGES, frustration=1.5)


def test_planted_sa_finds_ground():
    """SA on a small zero-frustration problem should find near the ground state."""
    from isingmark.annealers.sa_dimod import SADimodeAdapter

    gen = PlantedSolutionGenerator()
    nodes = list(range(8))
    edges = [(i, i + 1) for i in range(7)]
    problem = gen.generate(seed=42, nodes=nodes, edges=edges, frustration=0.0)
    known = gen.known_ground_energy(problem)
    assert known is not None

    adapter = SADimodeAdapter()
    sol = adapter.sample_ising(
        problem.h,
        problem.J,
        num_reads=200,
        num_sweeps=2000,
        seed=0,
    )
    # SA should find an energy at or above the planted ground energy (which is
    # negative) and within a generous gap.  known is the minimum achievable.
    assert sol.best_energy >= known  # cannot be lower than the planted ground
    assert sol.best_energy <= abs(known) * 0.5  # should be clearly negative
