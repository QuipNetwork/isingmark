# SPDX-License-Identifier: AGPL-3.0-or-later
"""Aggregate statistics across a collection of AnnealerSolution records."""
from __future__ import annotations

from typing import Any

import numpy as np

from isingmark.models import AnnealerSolution


def aggregate_solutions(solutions: list[AnnealerSolution]) -> dict[str, Any]:
    """Compute summary statistics across a list of AnnealerSolution records.

    Args:
        solutions: Collection of solution records to aggregate.

    Returns:
        Dict containing:
        - ``total_problems``: Number of solutions processed.
        - ``energy_stats``: min, max, mean, std, p25, p50, p75 of best_energy.
        - ``timing_stats``: min_s, max_s, mean_s, std_s of wall_time_s.
        - ``diversity_stats``: min, max, mean of diversity scores.
        For an empty input only ``total_problems: 0`` is returned.
    """
    if not solutions:
        return {"total_problems": 0}

    energies = np.array([s.best_energy for s in solutions], dtype=float)
    timings = np.array([s.wall_time_s for s in solutions], dtype=float)
    diversities = np.array([s.diversity for s in solutions], dtype=float)

    energy_stats: dict[str, float] = {
        "min": float(np.min(energies)),
        "max": float(np.max(energies)),
        "mean": float(np.mean(energies)),
        "std": float(np.std(energies)),
        "p25": float(np.percentile(energies, 25)),
        "p50": float(np.percentile(energies, 50)),
        "p75": float(np.percentile(energies, 75)),
    }

    timing_stats: dict[str, float] = {
        "min_s": float(np.min(timings)),
        "max_s": float(np.max(timings)),
        "mean_s": float(np.mean(timings)),
        "std_s": float(np.std(timings)),
    }

    diversity_stats: dict[str, float] = {
        "min": float(np.min(diversities)),
        "max": float(np.max(diversities)),
        "mean": float(np.mean(diversities)),
    }

    return {
        "total_problems": len(solutions),
        "energy_stats": energy_stats,
        "timing_stats": timing_stats,
        "diversity_stats": diversity_stats,
    }
