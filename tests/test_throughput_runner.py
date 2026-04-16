# SPDX-License-Identifier: AGPL-3.0-or-later
"""Integration test for throughput runner."""
from __future__ import annotations

from isingmark.annealers.sa_dimod import SADimodeAdapter
from isingmark.models import BenchmarkConfig
from isingmark.params.adaptor import DefaultParamsAdaptor
from isingmark.problems.random_ising import RandomIsingGenerator
from isingmark.runners.throughput import run_throughput


def test_throughput_5s():
    config = BenchmarkConfig(
        mode="throughput",
        annealer_name="sa-dimod",
        problem_type="random_ising",
        duration_s=5.0,
        hardness=0.3,
    )
    nodes = list(range(20))
    edges = [(i, i + 1) for i in range(19)]

    results = run_throughput(
        config=config,
        annealer=SADimodeAdapter(),
        params_adaptor=DefaultParamsAdaptor(),
        generator=RandomIsingGenerator(),
        nodes=nodes,
        edges=edges,
        seeds=range(1000),
        show_progress=False,
    )
    assert len(results) > 0
    for r in results:
        assert r.annealer_name == "sa-dimod"
        assert r.best_energy < 0
        assert r.wall_time_s > 0
