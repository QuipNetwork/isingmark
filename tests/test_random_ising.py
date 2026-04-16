# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for the random_ising problem generator and registry."""
from __future__ import annotations

import json
from pathlib import Path

import pytest

VECTORS_PATH = Path(__file__).parent / "chacha8_test_vectors.json"
VECTORS = json.loads(VECTORS_PATH.read_text())


def test_generator_registered() -> None:
    from isingmark.problems import get_generator

    gen = get_generator("random_ising")
    assert gen.name == "random_ising"


def test_generate_matches_test_vectors() -> None:
    from isingmark.problems import get_generator

    gen = get_generator("random_ising")

    for vec in VECTORS["generate_ising_model"]:
        seed = vec["nonce"]
        nodes: list[int] = vec["nodes"]
        edges: list[tuple[int, int]] = [tuple(e) for e in vec["edges"]]
        h_values: list[float] = vec["allowed_h_values"]
        expected_h: dict[str, float] = vec["expected_h"]
        expected_j: dict[str, float] = vec["expected_j"]

        problem = gen.generate(
            seed=seed,
            nodes=nodes,
            edges=edges,
            h_values=h_values,
        )

        # Verify h fields match expected values
        for node_str, expected_val in expected_h.items():
            node = int(node_str)
            assert problem.h[node] == pytest.approx(expected_val), (
                f"h[{node}] mismatch for seed={seed}, nodes={nodes}: "
                f"got {problem.h[node]}, expected {expected_val}"
            )

        # Verify J fields match expected values
        for edge_str, expected_val in expected_j.items():
            u_str, v_str = edge_str.split(",", 1)
            edge = (int(u_str), int(v_str))
            assert problem.J[edge] == pytest.approx(expected_val), (
                f"J[{edge}] mismatch for seed={seed}, nodes={nodes}: "
                f"got {problem.J[edge]}, expected {expected_val}"
            )


def test_deterministic_generation() -> None:
    from isingmark.problems import get_generator

    gen = get_generator("random_ising")
    nodes = [0, 1, 2, 3]
    edges = [(0, 1), (1, 2), (2, 3)]

    p1 = gen.generate(seed=99, nodes=nodes, edges=edges)
    p2 = gen.generate(seed=99, nodes=nodes, edges=edges)

    assert p1.h == p2.h
    assert p1.J == p2.J


def test_known_ground_energy_returns_none() -> None:
    from isingmark.problems import get_generator

    gen = get_generator("random_ising")
    nodes = [0, 1, 2]
    edges = [(0, 1), (1, 2)]
    problem = gen.generate(seed=42, nodes=nodes, edges=edges)

    assert gen.known_ground_energy(problem) is None


def test_list_generators_includes_random_ising() -> None:
    from isingmark.problems import list_generators

    assert "random_ising" in list_generators()


def test_get_generator_unknown_raises() -> None:
    from isingmark.problems import get_generator

    with pytest.raises(KeyError, match="Unknown problem type"):
        get_generator("nonexistent_generator")


def test_problem_has_correct_metadata() -> None:
    from isingmark.problems import get_generator

    gen = get_generator("random_ising")
    nodes = [0, 1, 2]
    edges = [(0, 1), (1, 2)]
    problem = gen.generate(seed=7, nodes=nodes, edges=edges)

    assert problem.seed == 7
    assert problem.problem_type == "random_ising"
    assert problem.nodes == nodes
    assert problem.edges == edges
