# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for isingmark.viz.tables."""

from isingmark.viz.tables import format_comparison_table, format_summary_table


def _make_summary(
    annealer: str = "sa-dimod",
    mode: str = "throughput",
    total: int = 100,
    rate: float = 12.5,
    min_e: float = -8.0,
    mean_e: float = -5.0,
    std_e: float = 1.2,
    mean_t: float = 0.05,
    std_t: float = 0.01,
) -> dict:
    return {
        "benchmark_mode": mode,
        "annealer_name": annealer,
        "problem_type": "random_ising",
        "total_problems": total,
        "problems_per_min": rate,
        "best_energy": {"min": min_e, "mean": mean_e, "max": -1.0, "std": std_e},
        "wall_time_s": {"mean": mean_t, "std": std_t},
    }


def test_format_summary_table_contains_key_values():
    summary = _make_summary()
    out = format_summary_table(summary)

    assert "throughput" in out
    assert "sa-dimod" in out
    assert "100" in out
    assert "12.50" in out
    assert "-8.0000" in out
    assert "-5.0000" in out


def test_format_summary_table_mode_and_rate():
    summary = _make_summary(mode="sweep", rate=0.0)
    out = format_summary_table(summary)
    assert "sweep" in out
    assert "0.00" in out


def test_format_comparison_table_header():
    summaries = [_make_summary("sa-dimod"), _make_summary("mock-annealer")]
    out = format_comparison_table(summaries)
    assert "Annealer" in out
    assert "Rate/min" in out
    assert "Min E" in out
    assert "Mean E" in out


def test_format_comparison_table_data_rows():
    summaries = [
        _make_summary("sa-dimod", total=50, rate=10.0, min_e=-9.0, mean_e=-6.0),
        _make_summary("mock-annealer", total=30, rate=5.0, min_e=-3.0, mean_e=-1.5),
    ]
    out = format_comparison_table(summaries)
    assert "sa-dimod" in out
    assert "mock-annealer" in out
    assert "50" in out
    assert "30" in out
    assert "-9.0000" in out
    assert "-3.0000" in out
