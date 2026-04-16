# SPDX-License-Identifier: AGPL-3.0-or-later
"""KS-distance comparator for cross-annealer energy distribution comparison."""
from __future__ import annotations

from collections.abc import Sequence

from scipy.stats import ks_2samp


def ks_distance(
    energies_a: Sequence[float],
    energies_b: Sequence[float],
) -> tuple[float, float]:
    """Compute the two-sample Kolmogorov-Smirnov distance between two energy distributions.

    Args:
        energies_a: Energy values from the first annealer/run.
        energies_b: Energy values from the second annealer/run.

    Returns:
        Tuple of (statistic, pvalue) where statistic is the KS distance in [0, 1]
        and pvalue is the two-sided p-value under the null hypothesis that both
        samples were drawn from the same continuous distribution.
    """
    result = ks_2samp(energies_a, energies_b)
    return float(result.statistic), float(result.pvalue)
