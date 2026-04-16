# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for JSONL result storage and summary JSON output."""
from __future__ import annotations

import json
import time

import pytest

from isingmark.models import AnnealerSolution
from isingmark.results.storage import (
    append_solution_jsonl,
    read_solutions_jsonl,
    write_summary_json,
)


def _make_solution(problem_id: str, best_energy: float) -> AnnealerSolution:
    return AnnealerSolution(
        problem_id=problem_id,
        annealer_name="sa-dimod",
        params={"num_reads": 10, "num_sweeps": 100},
        wall_time_s=0.123,
        best_energy=best_energy,
        mean_energy=best_energy + 1.0,
        std_energy=0.5,
        num_samples=10,
        energies=[best_energy + float(i) for i in range(10)],
        diversity=0.42,
        num_below_target=3,
        satisfaction_rate=0.8,
        hardware_timing={},
        timestamp=time.time(),
    )


def test_jsonl_roundtrip(tmp_path) -> None:
    sol_a = _make_solution("prob-A", -5.0)
    sol_b = _make_solution("prob-B", -10.0)
    out = tmp_path / "results.jsonl"

    append_solution_jsonl(out, sol_a)
    append_solution_jsonl(out, sol_b)

    loaded = read_solutions_jsonl(out)
    assert len(loaded) == 2

    assert loaded[0].problem_id == "prob-A"
    assert loaded[0].best_energy == pytest.approx(-5.0)
    assert loaded[0].annealer_name == "sa-dimod"
    assert loaded[0].num_samples == 10
    assert len(loaded[0].energies) == 10

    assert loaded[1].problem_id == "prob-B"
    assert loaded[1].best_energy == pytest.approx(-10.0)


def test_write_summary(tmp_path) -> None:
    summary = {
        "run_id": "test-run-001",
        "num_problems": 50,
        "mean_gap": 0.03,
        "annealer": "sa-dimod",
    }
    out = tmp_path / "summary.json"
    write_summary_json(out, summary)

    with open(out, encoding="utf-8") as fh:
        loaded = json.load(fh)

    assert loaded["run_id"] == "test-run-001"
    assert loaded["num_problems"] == 50
    assert loaded["mean_gap"] == pytest.approx(0.03)
    assert loaded["annealer"] == "sa-dimod"
