# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for KS-distance comparator."""
from __future__ import annotations

import pytest

from isingmark.results.comparator import ks_distance


def test_ks_identical() -> None:
    """Identical samples produce a near-zero KS statistic."""
    data = [-3.0, -2.5, -2.0, -1.5, -1.0]
    stat, pval = ks_distance(data, data)
    assert stat == pytest.approx(0.0, abs=1e-9)
    # p-value for identical samples is 1 (or undefined but scipy returns 1)
    assert pval >= 0.0


def test_ks_different() -> None:
    """Non-overlapping distributions produce a KS statistic near 1.0."""
    low = [-10.0, -9.0, -8.0, -7.0, -6.0]
    high = [6.0, 7.0, 8.0, 9.0, 10.0]
    stat, pval = ks_distance(low, high)
    assert stat == pytest.approx(1.0, abs=1e-9)
    assert pval < 0.05


def test_ks_partial_overlap() -> None:
    """Partially overlapping distributions produce 0 < stat < 1."""
    a = [-3.0, -2.0, -1.0, 0.0, 1.0]
    b = [-1.0, 0.0, 1.0, 2.0, 3.0]
    stat, pval = ks_distance(a, b)
    assert 0.0 < stat < 1.0


def test_ks_returns_floats() -> None:
    """Return types are plain Python floats."""
    stat, pval = ks_distance([1.0, 2.0], [3.0, 4.0])
    assert isinstance(stat, float)
    assert isinstance(pval, float)
