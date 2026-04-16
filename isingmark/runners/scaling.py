# SPDX-License-Identifier: AGPL-3.0-or-later
"""Scaling benchmark runner: measure annealer performance across problem sizes."""
from __future__ import annotations

import random

import click

from isingmark.annealers import AnnealerAdapter
from isingmark.models import AnnealerSolution, BenchmarkConfig
from isingmark.params.adaptor import ParamsAdaptor
from isingmark.problems import ProblemGenerator


def _make_random_graph(
    n: int,
    avg_degree: int,
    rng_seed: int,
) -> tuple[list[int], list[tuple[int, int]]]:
    """Generate a random graph with n nodes and approximately n*avg_degree/2 edges.

    Args:
        n: Number of nodes (0..n-1).
        avg_degree: Target average degree; determines target edge count.
        rng_seed: Seed for the random number generator.

    Returns:
        Tuple of (nodes, edges) where nodes is list[int] and edges is a list
        of (u, v) tuples with u < v (no self-loops, no duplicates).
    """
    rng = random.Random(rng_seed)
    nodes = list(range(n))
    target_edges = n * avg_degree // 2
    edges: set[tuple[int, int]] = set()
    while len(edges) < target_edges:
        u, v = rng.randrange(n), rng.randrange(n)
        if u != v:
            edges.add((min(u, v), max(u, v)))
    return nodes, list(edges)


def run_scaling(
    config: BenchmarkConfig,
    annealer: AnnealerAdapter,
    params_adaptor: ParamsAdaptor,
    generator: ProblemGenerator,
    sizes: list[int],
    avg_degree: int = 4,
    num_problems_per_size: int = 5,
    base_seed: int = 0,
    show_progress: bool = True,
) -> dict[int, list[AnnealerSolution]]:
    """Run the annealer on random graphs of increasing sizes.

    For each size a new random graph is generated and ``num_problems_per_size``
    Ising problems are solved with that graph's topology.

    Args:
        config: Benchmark configuration (mode, annealer_name, problem_type, …).
        annealer: Annealer adapter for sampling.
        params_adaptor: Maps config.hardness to annealer parameters.
        generator: Problem generator used to create Ising instances.
        sizes: List of node counts to benchmark.
        avg_degree: Average node degree for generated random graphs.
        num_problems_per_size: Number of problems to run per size.
        base_seed: Seed offset; each (size_idx, problem_idx) pair gets a unique
                   deterministic seed derived from this value.
        show_progress: Whether to display a click progress bar.

    Returns:
        Dict mapping each size (int) to a list of AnnealerSolution records.
    """
    params = params_adaptor.adapt(config.hardness, None)
    results: dict[int, list[AnnealerSolution]] = {}
    total = len(sizes) * num_problems_per_size

    if show_progress:
        bar_ctx = click.progressbar(
            length=total,
            label="Scaling",
            show_pos=True,
        )
    else:
        bar_ctx = _null_ctx()  # type: ignore[assignment]

    with bar_ctx as bar:
        for size_idx, size in enumerate(sizes):
            nodes, edges = _make_random_graph(size, avg_degree, base_seed + size_idx)
            solutions: list[AnnealerSolution] = []
            for prob_idx in range(num_problems_per_size):
                seed = base_seed + size_idx * num_problems_per_size + prob_idx
                problem = generator.generate(
                    seed=seed,
                    nodes=nodes,
                    edges=edges,
                    **config.problem_config,
                )
                sol = annealer.sample_ising(
                    problem.h,
                    problem.J,
                    problem_id=problem.problem_id,
                    nodes=problem.nodes,
                    **params,
                )
                solutions.append(sol)
                if show_progress:
                    bar.update(1)
            results[size] = solutions

    return results


class _null_ctx:
    """No-op context manager used when progress display is disabled."""

    def __enter__(self) -> "_null_ctx":
        return self

    def update(self, n: int) -> None:  # noqa: ARG002
        pass

    def __exit__(self, *args: object) -> None:
        pass
