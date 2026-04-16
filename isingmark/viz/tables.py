# SPDX-License-Identifier: AGPL-3.0-or-later
"""Text table formatters for benchmark summaries."""
from __future__ import annotations


def format_summary_table(summary: dict) -> str:
    """Format a summary dict as a readable text table.

    Renders key stats including benchmark_mode, total_problems,
    problems_per_minute, energy_stats, and timing_stats.

    Args:
        summary: Aggregated benchmark summary dict (as produced by the CLI or
                 aggregate_solutions).

    Returns:
        Multi-line string suitable for terminal output.
    """
    lines: list[str] = []
    lines.append("=" * 48)
    lines.append("  Benchmark Summary")
    lines.append("=" * 48)

    mode = summary.get("benchmark_mode", "N/A")
    annealer = summary.get("annealer_name", "N/A")
    problem_type = summary.get("problem_type", "N/A")
    total = summary.get("total_problems", 0)
    rate = summary.get("problems_per_min", 0.0)

    lines.append(f"  Mode            : {mode}")
    lines.append(f"  Annealer        : {annealer}")
    lines.append(f"  Problem type    : {problem_type}")
    lines.append(f"  Total problems  : {total}")
    lines.append(f"  Problems/min    : {rate:.2f}")

    energy = summary.get("best_energy") or summary.get("energy_stats")
    if energy:
        lines.append("-" * 48)
        lines.append("  Energy stats")
        lines.append(f"    min   : {energy.get('min', float('nan')):.4f}")
        lines.append(f"    mean  : {energy.get('mean', float('nan')):.4f}")
        lines.append(f"    max   : {energy.get('max', float('nan')):.4f}")
        std = energy.get("std", energy.get("std_energy"))
        if std is not None:
            lines.append(f"    std   : {std:.4f}")

    timing = summary.get("wall_time_s") or summary.get("timing_stats")
    if timing:
        lines.append("-" * 48)
        lines.append("  Timing stats")
        mean_key = "mean" if "mean" in timing else "mean_s"
        std_key = "std" if "std" in timing else "std_s"
        lines.append(f"    mean  : {timing.get(mean_key, float('nan')):.4f}s")
        lines.append(f"    std   : {timing.get(std_key, float('nan')):.4f}s")

    lines.append("=" * 48)
    return "\n".join(lines)


def format_comparison_table(summaries: list[dict]) -> str:
    """Format multiple summaries as a comparison table.

    Columns: Annealer, Problems, Rate/min, Min E, Mean E.

    Args:
        summaries: List of summary dicts, each as returned by the CLI or
                   aggregate_solutions. Each should have ``annealer_name``,
                   ``total_problems``, ``problems_per_min``, and an
                   energy stats sub-dict.

    Returns:
        Multi-line string with a header row and one data row per summary.
    """
    header = f"{'Annealer':<20} {'Problems':>10} {'Rate/min':>10} {'Min E':>12} {'Mean E':>12}"
    separator = "-" * len(header)
    lines: list[str] = [header, separator]

    for s in summaries:
        annealer = s.get("annealer_name", "N/A")
        total = s.get("total_problems", 0)
        rate = s.get("problems_per_min", 0.0)

        energy = s.get("best_energy") or s.get("energy_stats") or {}
        min_e = energy.get("min", float("nan"))
        mean_e = energy.get("mean", float("nan"))

        row = (
            f"{annealer:<20} {total:>10} {rate:>10.2f}"
            f" {min_e:>12.4f} {mean_e:>12.4f}"
        )
        lines.append(row)

    return "\n".join(lines)
