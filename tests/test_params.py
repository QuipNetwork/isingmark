# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for the DefaultParamsAdaptor."""
from __future__ import annotations

from isingmark.params.adaptor import DefaultParamsAdaptor


def _adaptor() -> DefaultParamsAdaptor:
    return DefaultParamsAdaptor(
        min_sweeps=64,
        max_sweeps=4096,
        min_reads=64,
        max_reads=512,
    )


def test_hardness_zero() -> None:
    params = _adaptor().adapt(0.0)
    assert params["num_sweeps"] == 64
    assert params["num_reads"] == 64


def test_hardness_one() -> None:
    params = _adaptor().adapt(1.0)
    assert params["num_sweeps"] == 4096
    assert params["num_reads"] == 512


def test_hardness_half() -> None:
    params = _adaptor().adapt(0.5)
    expected_sweeps = 64 + int(0.5 * (4096 - 64))
    expected_reads = 64 + int(0.5 * (512 - 64))
    assert params["num_sweeps"] == expected_sweeps
    assert params["num_reads"] == expected_reads


def test_hardness_clamped() -> None:
    adaptor = _adaptor()
    below = adaptor.adapt(-0.1)
    at_zero = adaptor.adapt(0.0)
    assert below["num_sweeps"] == at_zero["num_sweeps"]
    assert below["num_reads"] == at_zero["num_reads"]

    above = adaptor.adapt(1.5)
    at_one = adaptor.adapt(1.0)
    assert above["num_sweeps"] == at_one["num_sweeps"]
    assert above["num_reads"] == at_one["num_reads"]
