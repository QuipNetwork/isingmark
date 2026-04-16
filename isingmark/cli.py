# SPDX-License-Identifier: AGPL-3.0-or-later
"""CLI entry point for isingmark."""
from __future__ import annotations

import json
import re
import time
from pathlib import Path
from typing import Any

import click
import numpy as np

from isingmark.annealers import get_adapter
from isingmark.models import BenchmarkConfig
from isingmark.params.adaptor import DefaultParamsAdaptor
from isingmark.problems import get_generator
from isingmark.results.storage import append_solution_jsonl, read_solutions_jsonl, write_summary_json
from isingmark.runners.comparative import run_comparative
from isingmark.runners.sweep import run_sweep
from isingmark.runners.throughput import run_throughput
from isingmark.runners.ttt import run_ttt

_DURATION_RE = re.compile(r"^(\d+(?:\.\d+)?)\s*([smhSMH])$")
_UNIT_SECONDS = {"s": 1.0, "m": 60.0, "h": 3600.0}


def _parse_duration(s: str) -> float:
    """Parse a duration string like '30s', '5m', or '2h' to float seconds.

    Args:
        s: Duration string with a numeric value and unit suffix (s/m/h).

    Returns:
        Duration in seconds as a float.

    Raises:
        click.BadParameter: If the string does not match the expected format.
    """
    m = _DURATION_RE.match(s.strip())
    if not m:
        raise click.BadParameter(
            f"Invalid duration {s!r}. Use format like '30s', '5m', '2h'."
        )
    value, unit = float(m.group(1)), m.group(2).lower()
    return value * _UNIT_SECONDS[unit]


def _load_topology(path: str | None) -> tuple[list[int], list[tuple[int, int]]]:
    """Load topology from a JSON file, or generate a default 50-node chain graph.

    The JSON file should have keys "nodes" (list of ints) and "edges"
    (list of [u, v] pairs).

    Args:
        path: Path to a JSON topology file, or None for the default topology.

    Returns:
        Tuple of (nodes, edges).
    """
    if path is None:
        nodes = list(range(50))
        edges = [(i, i + 1) for i in range(49)]
        return nodes, edges

    data: dict[str, Any] = json.loads(Path(path).read_text(encoding="utf-8"))
    nodes = [int(n) for n in data["nodes"]]
    edges = [tuple(int(x) for x in e) for e in data["edges"]]  # type: ignore[misc]
    return nodes, edges  # type: ignore[return-value]


def _seed_sequence(base_seed: int) -> range:
    """Return a large seed range starting from base_seed.

    Args:
        base_seed: Starting seed value.

    Returns:
        range(base_seed, base_seed + 1_000_000).
    """
    return range(base_seed, base_seed + 1_000_000)


@click.group()
@click.version_option(version="0.1.0", prog_name="isingmark")
def main() -> None:
    """IsingMark: annealer-agnostic Ising model benchmarking."""


@main.command()
@click.option("--annealer", default="sa-dimod", show_default=True, help="Annealer adapter name.")
@click.option(
    "--problem-type", default="random_ising", show_default=True, help="Problem generator name."
)
@click.option(
    "--hardness", default=0.5, show_default=True, type=float, help="Problem hardness in [0, 1]."
)
@click.option("--duration", default="10m", show_default=True, help="Run duration (e.g. 30s, 5m, 2h).")
@click.option("--seed", default=0, show_default=True, type=int, help="Base seed for problem generation.")
@click.option(
    "--topology",
    default=None,
    type=click.Path(exists=True, dir_okay=False, readable=True),
    help="Path to JSON topology file with 'nodes' and 'edges' keys.",
)
@click.option(
    "--output-dir",
    default=".",
    show_default=True,
    type=click.Path(file_okay=False, writable=True),
    help="Directory for result files.",
)
@click.option("-v", "--verbose", is_flag=True, default=False, help="Show per-problem details.")
def throughput(
    annealer: str,
    problem_type: str,
    hardness: float,
    duration: str,
    seed: int,
    topology: str | None,
    output_dir: str,
    verbose: bool,
) -> None:
    """Run throughput benchmark: maximize problems/min at fixed hardness.

    Results are written to output_dir as:
      - results_<timestamp>.jsonl  (one solution per line)
      - summary_<timestamp>.json   (aggregate statistics)
    """
    duration_s = _parse_duration(duration)
    nodes, edges = _load_topology(topology)

    config = BenchmarkConfig(
        mode="throughput",
        annealer_name=annealer,
        problem_type=problem_type,
        hardness=hardness,
        duration_s=duration_s,
        output_dir=output_dir,
    )

    adapter = get_adapter(annealer)
    generator = get_generator(problem_type)
    params_adaptor = DefaultParamsAdaptor()

    click.echo(
        f"Running throughput benchmark: annealer={annealer}, "
        f"problem={problem_type}, hardness={hardness:.2f}, "
        f"duration={duration}, nodes={len(nodes)}, edges={len(edges)}"
    )

    results = run_throughput(
        config=config,
        annealer=adapter,
        params_adaptor=params_adaptor,
        generator=generator,
        nodes=nodes,
        edges=edges,
        seeds=_seed_sequence(seed),
        show_progress=not verbose,
    )

    if not results:
        click.echo("No problems solved within the time limit.", err=True)
        raise SystemExit(1)

    # Write JSONL results
    ts = int(time.time())
    out_path = Path(output_dir)
    out_path.mkdir(parents=True, exist_ok=True)
    jsonl_path = out_path / f"results_{ts}.jsonl"
    for sol in results:
        append_solution_jsonl(jsonl_path, sol)

    # Compute summary statistics
    energies = np.array([r.best_energy for r in results])
    wall_times = np.array([r.wall_time_s for r in results])
    total_wall = float(np.sum(wall_times))
    problems_per_min = len(results) / (duration_s / 60.0) if duration_s > 0 else 0.0

    summary: dict[str, Any] = {
        "benchmark_mode": "throughput",
        "annealer_name": annealer,
        "problem_type": problem_type,
        "hardness": hardness,
        "duration_s": duration_s,
        "total_problems": len(results),
        "problems_per_min": round(problems_per_min, 2),
        "best_energy": {"mean": float(np.mean(energies)), "std": float(np.std(energies)),
                        "min": float(np.min(energies)), "max": float(np.max(energies))},
        "wall_time_s": {"total": round(total_wall, 4), "mean": round(float(np.mean(wall_times)), 4),
                        "std": round(float(np.std(wall_times)), 4)},
        "results_file": str(jsonl_path),
    }

    summary_path = out_path / f"summary_{ts}.json"
    write_summary_json(summary_path, summary)

    # Print summary table
    click.echo(f"\nResults written to: {jsonl_path}")
    click.echo(f"Summary written to: {summary_path}\n")
    click.echo("--- Throughput Summary ---")
    click.echo(f"  Problems solved : {len(results)}")
    click.echo(f"  Problems/min    : {problems_per_min:.1f}")
    click.echo(f"  Best energy     : mean={summary['best_energy']['mean']:.4f}  "
               f"std={summary['best_energy']['std']:.4f}")
    click.echo(f"  Wall time/prob  : mean={summary['wall_time_s']['mean']:.4f}s  "
               f"std={summary['wall_time_s']['std']:.4f}s")

    if verbose:
        for i, r in enumerate(results):
            click.echo(
                f"  [{i:4d}] {r.problem_id[:8]}  energy={r.best_energy:.4f}  "
                f"wall={r.wall_time_s:.4f}s"
            )


@main.command()
@click.option("--annealer", default="sa-dimod", show_default=True, help="Annealer adapter name.")
@click.option(
    "--problem-type", default="random_ising", show_default=True, help="Problem generator name."
)
@click.option(
    "--param-grid",
    required=True,
    help='JSON param grid, e.g. \'{"num_reads":[64,128],"num_sweeps":[100,500]}\'',
)
@click.option(
    "--num-problems", default=10, show_default=True, type=int, help="Number of problems per combo."
)
@click.option("--seed", default=0, show_default=True, type=int, help="Base seed.")
@click.option(
    "--topology",
    default=None,
    type=click.Path(exists=True, dir_okay=False, readable=True),
    help="Path to JSON topology file.",
)
@click.option(
    "--output-dir",
    default=".",
    show_default=True,
    type=click.Path(file_okay=False, writable=True),
    help="Directory for result files.",
)
def sweep(
    annealer: str,
    problem_type: str,
    param_grid: str,
    num_problems: int,
    seed: int,
    topology: str | None,
    output_dir: str,
) -> None:
    """Run parameter sweep: grid search over annealer parameters.

    Results are written to output_dir as sweep_<timestamp>.jsonl.
    """
    try:
        grid: dict[str, list[Any]] = json.loads(param_grid)
    except json.JSONDecodeError as exc:
        raise click.BadParameter(f"Invalid JSON for --param-grid: {exc}") from exc

    nodes, edges = _load_topology(topology)
    adapter = get_adapter(annealer)
    generator = get_generator(problem_type)

    config = BenchmarkConfig(
        mode="sweep",
        annealer_name=annealer,
        problem_type=problem_type,
        output_dir=output_dir,
    )

    seeds = list(_seed_sequence(seed))
    problems = [
        generator.generate(seed=seeds[i % len(seeds)], nodes=nodes, edges=edges)
        for i in range(num_problems)
    ]

    click.echo(
        f"Running sweep: annealer={annealer}, problem={problem_type}, "
        f"num_problems={num_problems}, grid={grid}"
    )

    results = run_sweep(config=config, annealer=adapter, param_grid=grid, problems=problems)

    ts = int(time.time())
    out_path = Path(output_dir)
    out_path.mkdir(parents=True, exist_ok=True)
    jsonl_path = out_path / f"sweep_{ts}.jsonl"
    for sol in results:
        append_solution_jsonl(jsonl_path, sol)

    click.echo(f"\nSweep complete: {len(results)} results written to {jsonl_path}")


@main.command()
@click.option("--annealer", default="sa-dimod", show_default=True, help="Annealer adapter name.")
@click.option(
    "--problem-type", default="random_ising", show_default=True, help="Problem generator name."
)
@click.option(
    "--target-file",
    required=True,
    type=click.Path(exists=True),
    help="Path to target JSONL file.",
)
@click.option("--seed", default=0, show_default=True, type=int, help="Base seed.")
@click.option(
    "--topology",
    default=None,
    type=click.Path(exists=True, dir_okay=False, readable=True),
    help="Path to JSON topology file.",
)
@click.option(
    "--output-dir",
    default=".",
    show_default=True,
    type=click.Path(file_okay=False, writable=True),
    help="Directory for result files.",
)
@click.option(
    "--max-iterations",
    default=15,
    show_default=True,
    type=int,
    help="Binary search iterations.",
)
def comparative(
    annealer: str,
    problem_type: str,
    target_file: str,
    seed: int,
    topology: str | None,
    output_dir: str,
    max_iterations: int,
) -> None:
    """Run comparative benchmark: match energy distribution from target JSONL.

    Results are written to output_dir as comparative_<timestamp>.jsonl.
    """
    target_results = read_solutions_jsonl(target_file)
    if not target_results:
        click.echo("Target file is empty.", err=True)
        raise SystemExit(1)

    nodes, edges = _load_topology(topology)
    adapter = get_adapter(annealer)
    generator = get_generator(problem_type)
    params_adaptor = DefaultParamsAdaptor()

    config = BenchmarkConfig(
        mode="comparative",
        annealer_name=annealer,
        problem_type=problem_type,
        output_dir=output_dir,
    )

    click.echo(
        f"Running comparative: annealer={annealer}, problem={problem_type}, "
        f"target={target_file}, max_iterations={max_iterations}"
    )

    best_solutions, matched_hardness = run_comparative(
        config=config,
        annealer=adapter,
        target_results=target_results,
        params_adaptor=params_adaptor,
        generator=generator,
        nodes=nodes,
        edges=edges,
        seeds=_seed_sequence(seed),
        max_iterations=max_iterations,
    )

    ts = int(time.time())
    out_path = Path(output_dir)
    out_path.mkdir(parents=True, exist_ok=True)
    jsonl_path = out_path / f"comparative_{ts}.jsonl"
    for sol in best_solutions:
        append_solution_jsonl(jsonl_path, sol)

    click.echo(f"\nMatched hardness: {matched_hardness:.4f}")
    click.echo(f"Results written to: {jsonl_path}")


@main.command()
@click.option("--annealer", default="sa-dimod", show_default=True, help="Annealer adapter name.")
@click.option(
    "--problem-type", default="random_ising", show_default=True, help="Problem generator name."
)
@click.option(
    "--hardness", default=0.5, show_default=True, type=float, help="Problem hardness in [0, 1]."
)
@click.option(
    "--energy-targets",
    required=True,
    help="Comma-separated energy thresholds, e.g. '-5.0,-10.0,-15.0'.",
)
@click.option(
    "--num-trials", default=100, show_default=True, type=int, help="Trials per energy target."
)
@click.option(
    "--max-time",
    default=300.0,
    show_default=True,
    type=float,
    help="Max wall seconds per (target, trial) pair.",
)
@click.option("--seed", default=0, show_default=True, type=int, help="Base seed.")
@click.option(
    "--topology",
    default=None,
    type=click.Path(exists=True, dir_okay=False, readable=True),
    help="Path to JSON topology file.",
)
@click.option(
    "--output-dir",
    default=".",
    show_default=True,
    type=click.Path(file_okay=False, writable=True),
    help="Directory for result files.",
)
def ttt(
    annealer: str,
    problem_type: str,
    hardness: float,
    energy_targets: str,
    num_trials: int,
    max_time: float,
    seed: int,
    topology: str | None,
    output_dir: str,
) -> None:
    """Run time-to-target (TTT) benchmark.

    Results are written to output_dir as ttt_<timestamp>.json.
    """
    try:
        targets = [float(t.strip()) for t in energy_targets.split(",") if t.strip()]
    except ValueError as exc:
        raise click.BadParameter(f"Invalid --energy-targets: {exc}") from exc

    if not targets:
        raise click.BadParameter("--energy-targets must contain at least one value.")

    nodes, edges = _load_topology(topology)
    adapter = get_adapter(annealer)
    generator = get_generator(problem_type)
    params_adaptor = DefaultParamsAdaptor()

    config = BenchmarkConfig(
        mode="ttt",
        annealer_name=annealer,
        problem_type=problem_type,
        hardness=hardness,
        output_dir=output_dir,
    )

    click.echo(
        f"Running TTT: annealer={annealer}, problem={problem_type}, hardness={hardness:.2f}, "
        f"targets={targets}, num_trials={num_trials}"
    )

    ttt_results = run_ttt(
        config=config,
        annealer=adapter,
        params_adaptor=params_adaptor,
        generator=generator,
        nodes=nodes,
        edges=edges,
        energy_targets=targets,
        num_trials=num_trials,
        max_time_per_target=max_time,
        seeds=_seed_sequence(seed),
    )

    ts = int(time.time())
    out_path = Path(output_dir)
    out_path.mkdir(parents=True, exist_ok=True)

    summary: dict[str, Any] = {
        "benchmark_mode": "ttt",
        "annealer_name": annealer,
        "problem_type": problem_type,
        "hardness": hardness,
        "num_trials": num_trials,
        "max_time_per_target": max_time,
        "targets": {},
    }

    click.echo("\n--- TTT Summary ---")
    for target, times in ttt_results.items():
        success_rate = len(times) / num_trials if num_trials > 0 else 0.0
        mean_time = float(np.mean(times)) if times else float("nan")
        median_time = float(np.median(times)) if times else float("nan")
        summary["targets"][str(target)] = {
            "success_count": len(times),
            "success_rate": round(success_rate, 4),
            "mean_time_s": round(mean_time, 4) if times else None,
            "median_time_s": round(median_time, 4) if times else None,
        }
        click.echo(
            f"  target={target:8.3f}  successes={len(times)}/{num_trials}  "
            f"success_rate={success_rate:.2%}  "
            f"mean_time={'N/A' if not times else f'{mean_time:.4f}s'}"
        )

    ttt_path = out_path / f"ttt_{ts}.json"
    write_summary_json(ttt_path, summary)
    click.echo(f"\nTTT summary written to: {ttt_path}")


if __name__ == "__main__":
    main()
