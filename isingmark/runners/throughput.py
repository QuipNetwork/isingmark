# SPDX-License-Identifier: AGPL-3.0-or-later
"""Throughput benchmark runner: problems/min at fixed hardness."""
from __future__ import annotations

import time
from collections.abc import Iterable

import click

from isingmark.annealers import AnnealerAdapter
from isingmark.models import AnnealerSolution, BenchmarkConfig
from isingmark.params.adaptor import ParamsAdaptor
from isingmark.problems import ProblemGenerator


def run_throughput(
    *,
    config: BenchmarkConfig,
    annealer: AnnealerAdapter,
    params_adaptor: ParamsAdaptor,
    generator: ProblemGenerator,
    nodes: list[int],
    edges: list[tuple[int, int]],
    seeds: Iterable[int],
    show_progress: bool = True,
) -> list[AnnealerSolution]:
    """Run annealer for config.duration_s, return all solutions.

    Args:
        config: Benchmark configuration including duration_s and hardness.
        annealer: Annealer adapter to use for sampling.
        params_adaptor: Maps hardness to annealer parameters.
        generator: Problem generator for creating Ising instances.
        nodes: List of node identifiers in the topology.
        edges: List of (u, v) edge tuples in the topology.
        seeds: Iterable of seeds for problem generation.
        show_progress: Whether to show a click progress bar.

    Returns:
        List of AnnealerSolution for all problems solved within the time limit.
    """
    params = params_adaptor.adapt(config.hardness, None)
    results: list[AnnealerSolution] = []
    seed_iter = iter(seeds)
    deadline = time.monotonic() + config.duration_s

    # Time-based progress bar
    if show_progress:
        bar = click.progressbar(
            length=int(config.duration_s),
            label="Throughput",
            show_eta=True,
            show_pos=True,
        )
    else:
        bar = None

    start = time.monotonic()
    last_tick = start

    try:
        if bar is not None:
            bar.__enter__()

        while time.monotonic() < deadline:
            seed = next(seed_iter, None)
            if seed is None:
                break

            problem = generator.generate(
                seed=seed, nodes=nodes, edges=edges,
                **config.problem_config,
            )

            solution = annealer.sample_ising(
                problem.h,
                problem.J,
                num_reads=params["num_reads"],
                num_sweeps=params["num_sweeps"],
                problem_id=problem.problem_id,
                nodes=problem.nodes,
            )
            results.append(solution)

            if bar is not None:
                now = time.monotonic()
                elapsed = int(now - start)
                delta = int(now - last_tick)
                if delta > 0:
                    bar.update(delta)
                    last_tick = now
    finally:
        if bar is not None:
            bar.__exit__(None, None, None)

    return results
