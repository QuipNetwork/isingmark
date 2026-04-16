# SPDX-License-Identifier: AGPL-3.0-or-later
"""Comparative runner: binary search over hardness to match a target energy distribution."""
from __future__ import annotations

import statistics
from collections.abc import Sequence
from typing import Any

import click

from isingmark.annealers import AnnealerAdapter
from isingmark.models import AnnealerSolution, BenchmarkConfig
from isingmark.params.adaptor import ParamsAdaptor
from isingmark.problems import ProblemGenerator
from isingmark.results.comparator import ks_distance


def run_comparative(
    config: BenchmarkConfig,
    annealer: AnnealerAdapter,
    target_results: list[AnnealerSolution],
    params_adaptor: ParamsAdaptor,
    generator: ProblemGenerator,
    nodes: list[int],
    edges: list[tuple[int, int]],
    seeds: Sequence[int],
    max_iterations: int = 15,
    num_problems_per_step: int = 10,
    show_progress: bool = True,
) -> tuple[list[AnnealerSolution], float]:
    """Binary-search over hardness in [0, 1] to find the value whose energy
    distribution best matches ``target_results``.

    Algorithm:
    1. Maintain ``lo=0``, ``hi=1``.  Repeat ``max_iterations`` times:
       a. ``mid = (lo + hi) / 2``
       b. Generate ``num_problems_per_step`` solutions at hardness ``mid``.
       c. Compute KS distance between candidate and target best-energies.
       d. If candidate mean energy > target mean energy → ``lo = mid`` (need
          more effort), else ``hi = mid``.
    2. Track ``best_hardness`` as the ``mid`` with the lowest KS statistic seen.

    Args:
        config: Benchmark configuration (mutated in place for hardness).
        annealer: Annealer adapter.
        target_results: Reference solutions whose energy distribution is the goal.
        params_adaptor: Maps hardness to annealer parameters.
        generator: Problem generator.
        nodes: Node list.
        edges: Edge list.
        seeds: Seed pool; cycled if fewer than needed.
        max_iterations: Number of binary-search iterations.
        num_problems_per_step: Problems sampled per iteration.
        show_progress: Display click progress bar.

    Returns:
        Tuple of (best_solutions, best_hardness) where best_solutions are the
        AnnealerSolution records from the iteration that achieved the lowest KS
        distance to target.
    """
    target_energies = [s.best_energy for s in target_results]
    target_mean = statistics.mean(target_energies) if target_energies else 0.0

    lo, hi = 0.0, 1.0
    best_hardness = 0.5
    best_ks = float("inf")
    best_solutions: list[AnnealerSolution] = []

    seed_list = list(seeds)
    seed_count = len(seed_list)

    if show_progress:
        bar_ctx: Any = click.progressbar(
            range(max_iterations),
            label="Comparative search",
            show_pos=True,
        )
    else:
        bar_ctx = _range_ctx(max_iterations)

    with bar_ctx as iterations:
        for i in iterations:
            mid = (lo + hi) / 2.0
            params = params_adaptor.adapt(mid, None)

            candidate_solutions: list[AnnealerSolution] = []
            for j in range(num_problems_per_step):
                seed = seed_list[(i * num_problems_per_step + j) % max(seed_count, 1)]
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
                candidate_solutions.append(sol)

            candidate_energies = [s.best_energy for s in candidate_solutions]
            ks_stat, _ = ks_distance(candidate_energies, target_energies)

            if ks_stat < best_ks:
                best_ks = ks_stat
                best_hardness = mid
                best_solutions = candidate_solutions

            candidate_mean = statistics.mean(candidate_energies)
            if candidate_mean > target_mean:
                lo = mid  # candidate is too easy (higher energies) → increase hardness
            else:
                hi = mid  # candidate is too hard (lower energies) → decrease hardness

    return best_solutions, best_hardness


class _range_ctx:
    """Simple context manager wrapping range() for no-progress mode."""

    def __init__(self, n: int) -> None:
        self._n = n

    def __enter__(self) -> range:
        return range(self._n)

    def __exit__(self, *args: Any) -> None:
        pass
