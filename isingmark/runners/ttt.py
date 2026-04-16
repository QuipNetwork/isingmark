# SPDX-License-Identifier: AGPL-3.0-or-later
"""Time-to-target (TTT) runner: measure how long it takes to reach energy thresholds."""
from __future__ import annotations

import time
from collections.abc import Sequence
from typing import Any

import click

from isingmark.annealers import AnnealerAdapter
from isingmark.models import BenchmarkConfig
from isingmark.params.adaptor import ParamsAdaptor
from isingmark.problems import ProblemGenerator


def run_ttt(
    config: BenchmarkConfig,
    annealer: AnnealerAdapter,
    params_adaptor: ParamsAdaptor,
    generator: ProblemGenerator,
    nodes: list[int],
    edges: list[tuple[int, int]],
    energy_targets: list[float],
    num_trials: int = 100,
    max_time_per_target: float = 300.0,
    seeds: Sequence[int] | None = None,
    show_progress: bool = True,
) -> dict[float, list[float]]:
    """Measure time-to-target for each energy threshold.

    For each target energy and each trial:
    1. Generate a new Ising problem (cycling through ``seeds`` if provided).
    2. Repeatedly call the annealer until ``best_energy <= target`` or
       ``max_time_per_target`` seconds elapse.
    3. Record the wall-clock time if the target was reached.

    Args:
        config: Benchmark configuration (hardness used for initial params).
        annealer: Annealer adapter.
        params_adaptor: Maps hardness to annealer parameters.
        generator: Problem generator.
        nodes: Node list.
        edges: Edge list.
        energy_targets: List of energy thresholds to test.
        num_trials: Number of independent trials per target.
        max_time_per_target: Wall-clock budget per (target, trial) pair in seconds.
        seeds: Optional seed pool; cycled by trial index if provided.
        show_progress: Whether to show a click progress bar.

    Returns:
        Dict mapping each target energy to a list of successful trial times (in
        seconds).  Trials that exceeded ``max_time_per_target`` are omitted, so
        the list may be shorter than ``num_trials``.
    """
    params = params_adaptor.adapt(config.hardness, None)
    seed_list = list(seeds) if seeds is not None else None
    seed_count = len(seed_list) if seed_list else 0

    total = len(energy_targets) * num_trials
    results: dict[float, list[float]] = {t: [] for t in energy_targets}

    if show_progress:
        bar_ctx: Any = click.progressbar(
            length=total,
            label="TTT",
            show_pos=True,
            show_eta=True,
        )
    else:
        bar_ctx = _NoOpBar()

    with bar_ctx as bar:
        for target in energy_targets:
            for trial_idx in range(num_trials):
                seed: int | None = None
                if seed_list:
                    seed = seed_list[trial_idx % seed_count]

                problem = generator.generate(
                    seed=seed if seed is not None else trial_idx,
                    nodes=nodes,
                    edges=edges,
                    **config.problem_config,
                )

                trial_start = time.monotonic()
                deadline = trial_start + max_time_per_target
                reached = False

                while time.monotonic() < deadline:
                    sample_kwargs: dict[str, Any] = dict(params)
                    if seed is not None:
                        sample_kwargs["seed"] = seed
                    sol = annealer.sample_ising(
                        problem.h,
                        problem.J,
                        problem_id=problem.problem_id,
                        nodes=problem.nodes,
                        **sample_kwargs,
                    )
                    if sol.best_energy <= target:
                        elapsed = time.monotonic() - trial_start
                        results[target].append(elapsed)
                        reached = True
                        break

                if show_progress and bar is not None:
                    bar.update(1)

    return results


class _NoOpBar:
    """No-op context manager used when show_progress=False."""

    def __enter__(self) -> None:
        return None

    def __exit__(self, *args: Any) -> None:
        pass
