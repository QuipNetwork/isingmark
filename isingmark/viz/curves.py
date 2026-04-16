# SPDX-License-Identifier: AGPL-3.0-or-later
"""CDF and distribution curve visualizations (optional matplotlib)."""
from __future__ import annotations

from pathlib import Path

try:
    import matplotlib.pyplot as plt  # type: ignore[import]
    import numpy as np  # type: ignore[import]

    _HAS_MATPLOTLIB = True
except ImportError:
    _HAS_MATPLOTLIB = False


def plot_ttt_cdf(
    ttt_results: dict[float, list[float]],
    output_path: str | Path | None = None,
):
    """Plot empirical CDFs of time-to-target for multiple energy targets.

    Args:
        ttt_results: Mapping from energy threshold to list of times-to-reach
                     that threshold across trials.  Trials that never reached
                     the threshold should be omitted from the list.
        output_path: If given, save the figure to this path.

    Returns:
        matplotlib Figure if matplotlib is available, else None.

    Raises:
        ImportError: If matplotlib is not installed.
    """
    if not _HAS_MATPLOTLIB:
        raise ImportError(
            "matplotlib is required for plot_ttt_cdf. "
            "Install it with: pip install matplotlib"
        )

    fig, ax = plt.subplots(figsize=(8, 5))

    for target in sorted(ttt_results):
        times = sorted(ttt_results[target])
        if not times:
            continue
        n = len(times)
        cdf = np.arange(1, n + 1) / n
        ax.step(times, cdf, where="post", label=f"target={target:.2f}")

    ax.set_xlabel("Time to target (s)")
    ax.set_ylabel("CDF")
    ax.set_title("Time-to-Target CDF")
    ax.legend(loc="lower right")
    ax.set_xlim(left=0.0)
    ax.set_ylim(0.0, 1.05)

    if output_path is not None:
        fig.savefig(str(output_path), bbox_inches="tight")
    return fig


def plot_energy_distributions(
    energy_sets: dict[str, list[float]],
    output_path: str | Path | None = None,
):
    """Plot overlapping histograms of energy distributions for multiple annealers.

    Args:
        energy_sets: Mapping from annealer/run label to list of energy values.
        output_path: If given, save the figure to this path.

    Returns:
        matplotlib Figure if matplotlib is available, else None.

    Raises:
        ImportError: If matplotlib is not installed.
    """
    if not _HAS_MATPLOTLIB:
        raise ImportError(
            "matplotlib is required for plot_energy_distributions. "
            "Install it with: pip install matplotlib"
        )

    fig, ax = plt.subplots(figsize=(8, 5))

    for label, energies in energy_sets.items():
        if not energies:
            continue
        ax.hist(energies, bins="auto", alpha=0.5, label=label, density=True)

    ax.set_xlabel("Energy")
    ax.set_ylabel("Density")
    ax.set_title("Energy Distributions")
    ax.legend()

    if output_path is not None:
        fig.savefig(str(output_path), bbox_inches="tight")
    return fig
