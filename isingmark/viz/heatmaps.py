# SPDX-License-Identifier: AGPL-3.0-or-later
"""Heatmap visualization for parameter sweep results (optional matplotlib)."""
from __future__ import annotations

from pathlib import Path
from typing import Any

try:
    import matplotlib.pyplot as plt  # type: ignore[import]
    import numpy as np  # type: ignore[import]

    _HAS_MATPLOTLIB = True
except ImportError:
    _HAS_MATPLOTLIB = False


def plot_sweep_heatmap(
    results: list[dict[str, Any]],
    x_param: str,
    y_param: str,
    metric: str = "best_energy",
    output_path: str | Path | None = None,
):
    """Plot a 2-D heatmap of a metric across two sweep parameters.

    Each cell is the mean of all result records that share the same
    (x_param, y_param) combination.

    Args:
        results: List of result dicts; each must contain ``x_param``,
                 ``y_param``, and ``metric`` keys.
        x_param: Name of the parameter mapped to the x-axis.
        y_param: Name of the parameter mapped to the y-axis.
        metric: Key of the scalar metric to visualise (default ``best_energy``).
        output_path: If given, save the figure to this path instead of showing.

    Returns:
        matplotlib Figure if matplotlib is available, else None.

    Raises:
        ImportError: If matplotlib is not installed.
    """
    if not _HAS_MATPLOTLIB:
        raise ImportError(
            "matplotlib is required for plot_sweep_heatmap. "
            "Install it with: pip install matplotlib"
        )

    x_vals = sorted({r[x_param] for r in results})
    y_vals = sorted({r[y_param] for r in results})
    x_idx = {v: i for i, v in enumerate(x_vals)}
    y_idx = {v: i for i, v in enumerate(y_vals)}

    grid = np.full((len(y_vals), len(x_vals)), float("nan"))
    counts = np.zeros((len(y_vals), len(x_vals)), dtype=int)

    for r in results:
        xi = x_idx[r[x_param]]
        yi = y_idx[r[y_param]]
        val = float(r[metric])
        if np.isnan(grid[yi, xi]):
            grid[yi, xi] = val
        else:
            grid[yi, xi] = (grid[yi, xi] * counts[yi, xi] + val) / (counts[yi, xi] + 1)
        counts[yi, xi] += 1

    fig, ax = plt.subplots(figsize=(max(4, len(x_vals)), max(3, len(y_vals))))
    im = ax.imshow(grid, aspect="auto", origin="lower")
    fig.colorbar(im, ax=ax, label=metric)

    ax.set_xticks(range(len(x_vals)))
    ax.set_xticklabels([str(v) for v in x_vals])
    ax.set_yticks(range(len(y_vals)))
    ax.set_yticklabels([str(v) for v in y_vals])
    ax.set_xlabel(x_param)
    ax.set_ylabel(y_param)
    ax.set_title(f"{metric} heatmap: {x_param} vs {y_param}")

    if output_path is not None:
        fig.savefig(str(output_path), bbox_inches="tight")
    return fig
