# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for the MAX-CUT problem generator."""
from __future__ import annotations

import isingmark.problems  # ensure registration
from isingmark.problems import list_generators
from isingmark.problems.max_cut import MaxCutGenerator


NODES = list(range(10))
EDGES = [(i, i + 1) for i in range(9)]


def test_max_cut_registered():
    """MaxCutGenerator must be in the global registry."""
    assert "max_cut" in list_generators()


def test_max_cut_h_is_zero():
    """h must be 0.0 for every node."""
    gen = MaxCutGenerator()
    problem = gen.generate(seed=0, nodes=NODES, edges=EDGES)
    assert all(v == 0.0 for v in problem.h.values())
    assert set(problem.h.keys()) == set(NODES)


def test_max_cut_j_values():
    """All J values must be in {-1.0, +1.0}."""
    gen = MaxCutGenerator()
    problem = gen.generate(seed=0, nodes=NODES, edges=EDGES)
    assert all(v in (-1.0, 1.0) for v in problem.J.values())
    assert len(problem.J) == len(EDGES)


def test_max_cut_deterministic():
    """Same seed must produce the same problem."""
    gen = MaxCutGenerator()
    p1 = gen.generate(seed=123, nodes=NODES, edges=EDGES)
    p2 = gen.generate(seed=123, nodes=NODES, edges=EDGES)
    assert p1.h == p2.h
    assert p1.J == p2.J


def test_max_cut_different_seeds_differ():
    """Different seeds should give different J assignments."""
    gen = MaxCutGenerator()
    p1 = gen.generate(seed=0, nodes=NODES, edges=EDGES)
    p2 = gen.generate(seed=1, nodes=NODES, edges=EDGES)
    # With 9 edges, the probability of identical J by chance is 2^-9 ≈ 0.2%
    assert p1.J != p2.J


def test_max_cut_known_ground_energy_is_none():
    """known_ground_energy must return None."""
    gen = MaxCutGenerator()
    problem = gen.generate(seed=0, nodes=NODES, edges=EDGES)
    assert gen.known_ground_energy(problem) is None


def test_max_cut_problem_type():
    """problem_type field must equal 'max_cut'."""
    gen = MaxCutGenerator()
    problem = gen.generate(seed=0, nodes=NODES, edges=EDGES)
    assert problem.problem_type == "max_cut"
