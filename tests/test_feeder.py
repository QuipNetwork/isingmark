# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for ProblemFeeder."""
from __future__ import annotations

import pytest

from isingmark.problems import get_generator
from isingmark.problems.feeder import ProblemFeeder


NODES = list(range(4))
EDGES = [(0, 1), (1, 2), (2, 3)]


def _make_feeder(seeds: list[int], buffer_size: int = 4, max_workers: int = 2) -> ProblemFeeder:
    gen = get_generator("random_ising")
    return ProblemFeeder(
        gen,
        nodes=NODES,
        edges=EDGES,
        seeds=seeds,
        buffer_size=buffer_size,
        max_workers=max_workers,
    )


def test_feeder_produces_problems() -> None:
    """Pop 5 problems from a feeder seeded with 10 different seeds."""
    seeds = list(range(10))
    feeder = _make_feeder(seeds)
    try:
        problems = [feeder.pop_blocking() for _ in range(5)]
    finally:
        feeder.stop()

    assert len(problems) == 5
    # All problem IDs should be unique (uuid4 is used internally)
    ids = {p.problem_id for p in problems}
    assert len(ids) == 5, "Expected 5 distinct problem_ids"
    # All should have the right topology
    for p in problems:
        assert p.nodes == NODES
        assert p.edges == EDGES


def test_feeder_deterministic() -> None:
    """Same seeds produce byte-identical h and J fields."""
    seeds = [42, 99]
    feeder_a = _make_feeder(seeds)
    feeder_b = _make_feeder(seeds)
    try:
        probs_a = [feeder_a.pop_blocking() for _ in seeds]
        probs_b = [feeder_b.pop_blocking() for _ in seeds]
    finally:
        feeder_a.stop()
        feeder_b.stop()

    for a, b in zip(probs_a, probs_b):
        assert a.h == b.h, "h fields should be identical for same seed"
        assert a.J == b.J, "J fields should be identical for same seed"
        assert a.seed == b.seed


def test_feeder_try_pop_empty() -> None:
    """With no seeds, try_pop returns None."""
    feeder = _make_feeder(seeds=[])
    try:
        result = feeder.try_pop()
    finally:
        feeder.stop()

    assert result is None


def test_feeder_stop_iteration() -> None:
    """pop_blocking raises StopIteration after all seeds are consumed."""
    seeds = [1, 2]
    feeder = _make_feeder(seeds)
    try:
        feeder.pop_blocking()
        feeder.pop_blocking()
        with pytest.raises(StopIteration):
            feeder.pop_blocking()
    finally:
        feeder.stop()


def test_feeder_iterator_protocol() -> None:
    """Feeder works as a for-loop iterator."""
    seeds = [10, 20, 30]
    feeder = _make_feeder(seeds)
    try:
        collected = list(feeder)
    finally:
        feeder.stop()

    assert len(collected) == 3
    for p in collected:
        assert p.problem_type == "random_ising"
