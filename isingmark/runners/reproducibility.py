# SPDX-License-Identifier: AGPL-3.0-or-later
"""Reproducibility runner: verify energy distributions are stable across identical runs."""
from __future__ import annotations

from collections.abc import Sequence

import click

from isingmark.annealers import AnnealerAdapter
from isingmark.models import AnnealerSolution, BenchmarkConfig
from isingmark.params.adaptor import ParamsAdaptor
from isingmark.problems import ProblemGenerator
from isingmark.results.comparator import ks_distance


def run_reproducibility(
    config: BenchmarkConfig,
    annealer: AnnealerAdapter,
    params_adaptor: ParamsAdaptor,
    generator: ProblemGenerator,
    nodes: list[int],
    edges: list[tuple[int, int]],
    seeds: Sequence[int],
    show_progress: bool = True,
) -> dict:
    """Run the same set of problems twice and compare the resulting energy distributions.

    The two runs use identical topology, seeds, and parameters.  A
    Kolmogorov-Smirnov test is used to assess whether the distributions are
    statistically indistinguishable.

    Args:
        config: Benchmark configuration; hardness is used to derive params.
        annealer: Annealer adapter for sampling.
        params_adaptor: Maps hardness to annealer parameters.
        generator: Problem generator to create Ising instances.
        nodes: List of node identifiers for the problem topology.
        edges: List of (u, v) edge tuples for the problem topology.
        seeds: Sequence of seeds; one problem per seed per run.
        show_progress: Whether to display a click progress bar.

    Returns:
        Dict with keys:
        - ``ks_statistic`` (float): KS test statistic.
        - ``p_value`` (float): Two-sided p-value.
        - ``consistent`` (bool): True when p_value > 0.05.
        - ``num_problems`` (int): Number of problems solved per run.
        - ``run1_results`` (list[AnnealerSolution]): First-run solutions.
        - ``run2_results`` (list[AnnealerSolution]): Second-run solutions.
    """
    params = params_adaptor.adapt(config.hardness, None)
    seed_list = list(seeds)
    num_problems = len(seed_list)

    total_steps = num_problems * 2

    if show_progress:
        bar_ctx = click.progressbar(
            length=total_steps,
            label="Reproducibility",
            show_pos=True,
        )
    else:
        bar_ctx = _null_ctx()  # type: ignore[assignment]

    run1: list[AnnealerSolution] = []
    run2: list[AnnealerSolution] = []

    with bar_ctx as bar:
        for seed in seed_list:
            problem = generator.generate(seed=seed, nodes=nodes, edges=edges)
            sol = annealer.sample_ising(
                problem.h,
                problem.J,
                problem_id=problem.problem_id,
                nodes=problem.nodes,
                **params,
            )
            run1.append(sol)
            if show_progress:
                bar.update(1)

        for seed in seed_list:
            problem = generator.generate(seed=seed, nodes=nodes, edges=edges)
            sol = annealer.sample_ising(
                problem.h,
                problem.J,
                problem_id=problem.problem_id,
                nodes=problem.nodes,
                **params,
            )
            run2.append(sol)
            if show_progress:
                bar.update(1)

    energies1 = [s.best_energy for s in run1]
    energies2 = [s.best_energy for s in run2]

    ks_stat, p_val = ks_distance(energies1, energies2)

    return {
        "ks_statistic": ks_stat,
        "p_value": p_val,
        "consistent": p_val > 0.05,
        "num_problems": num_problems,
        "run1_results": run1,
        "run2_results": run2,
    }


class _null_ctx:
    """No-op context manager used when progress display is disabled."""

    def __enter__(self) -> "_null_ctx":
        return self

    def update(self, n: int) -> None:  # noqa: ARG002
        pass

    def __exit__(self, *args: object) -> None:
        pass
