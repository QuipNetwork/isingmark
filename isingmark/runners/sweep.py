# SPDX-License-Identifier: AGPL-3.0-or-later
"""Parameter sweep runner: grid search over annealer parameters."""
from __future__ import annotations

import itertools
from typing import Any

import click

from isingmark.annealers import AnnealerAdapter
from isingmark.models import AnnealerSolution, BenchmarkConfig, IsingProblem


def run_sweep(
    config: BenchmarkConfig,
    annealer: AnnealerAdapter,
    param_grid: dict[str, list[Any]],
    problems: list[IsingProblem],
    show_progress: bool = True,
) -> list[AnnealerSolution]:
    """Run a grid search over annealer parameters across a set of problems.

    For every combination of values in ``param_grid`` and every problem in
    ``problems``, calls ``annealer.sample_ising`` with those parameters and
    records the result.

    Args:
        config: Benchmark configuration (currently informational; not mutated).
        annealer: Annealer adapter used for sampling.
        param_grid: Dict mapping parameter names to lists of values to try.
            E.g. ``{"num_reads": [10, 50], "num_sweeps": [100, 500]}``.
        problems: List of IsingProblem instances to solve under each combo.
        show_progress: Whether to display a click progress bar.

    Returns:
        List of AnnealerSolution in the order (combo_0, problem_0),
        (combo_0, problem_1), …, (combo_n, problem_m).
    """
    param_names = list(param_grid.keys())
    value_lists = [param_grid[k] for k in param_names]
    combos = list(itertools.product(*value_lists))

    all_tasks: list[tuple[dict[str, Any], IsingProblem]] = [
        (dict(zip(param_names, combo)), problem)
        for combo in combos
        for problem in problems
    ]

    results: list[AnnealerSolution] = []

    if show_progress:
        bar_ctx = click.progressbar(
            all_tasks,
            label="Sweep",
            show_pos=True,
            show_eta=True,
        )
    else:
        bar_ctx = _NoOpContext(all_tasks)

    with bar_ctx as tasks:
        for params, problem in tasks:
            solution = annealer.sample_ising(
                problem.h,
                problem.J,
                problem_id=problem.problem_id,
                nodes=problem.nodes,
                **params,
            )
            results.append(solution)

    return results


class _NoOpContext:
    """Minimal context manager that iterates over items without decoration."""

    def __init__(self, items: list[Any]) -> None:
        self._items = items

    def __enter__(self) -> list[Any]:
        return self._items

    def __exit__(self, *args: Any) -> None:
        pass
