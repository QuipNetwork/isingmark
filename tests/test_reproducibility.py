# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for isingmark.runners.reproducibility."""

from isingmark.annealers import get_adapter
from isingmark.models import BenchmarkConfig
from isingmark.params.adaptor import DefaultParamsAdaptor
from isingmark.problems import get_generator
from isingmark.runners.reproducibility import run_reproducibility


def _chain_topology(n: int) -> tuple[list[int], list[tuple[int, int]]]:
    nodes = list(range(n))
    edges = [(i, i + 1) for i in range(n - 1)]
    return nodes, edges


def test_reproducibility_consistent():
    """Two runs of the same SA problems should produce consistent energy distributions."""
    nodes, edges = _chain_topology(15)
    seeds = list(range(10))

    config = BenchmarkConfig(
        mode="reproducibility",
        annealer_name="sa-dimod",
        problem_type="random_ising",
        hardness=0.3,
    )
    annealer = get_adapter("sa-dimod")
    params_adaptor = DefaultParamsAdaptor()
    generator = get_generator("random_ising")

    report = run_reproducibility(
        config=config,
        annealer=annealer,
        params_adaptor=params_adaptor,
        generator=generator,
        nodes=nodes,
        edges=edges,
        seeds=seeds,
        show_progress=False,
    )

    # Verify all expected keys are present
    assert "ks_statistic" in report
    assert "p_value" in report
    assert "consistent" in report
    assert "num_problems" in report
    assert "run1_results" in report
    assert "run2_results" in report

    assert report["num_problems"] == len(seeds)
    assert len(report["run1_results"]) == len(seeds)
    assert len(report["run2_results"]) == len(seeds)

    # KS statistic is in [0, 1]
    assert 0.0 <= report["ks_statistic"] <= 1.0
    # p_value is non-negative
    assert report["p_value"] >= 0.0
    # consistent flag matches the p-value threshold
    assert report["consistent"] == (report["p_value"] > 0.05)
