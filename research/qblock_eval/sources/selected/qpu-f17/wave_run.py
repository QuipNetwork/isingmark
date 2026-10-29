#!/usr/bin/env python3
"""Run one variant wave end to end: models, production capture, GPU ladder, cluster ladder, score.

A wave is a cell built from the suite's `ice-twisted` models (weak layer rescaled, pairs
planted on weak couplers at a chosen strength), a cohort of nonces, a GPU single-flip ladder
and a round-2 cluster-solver ladder. The steps are idempotent: models, captures, GPU result
files and cluster measurement directories that already exist are reused.

    wave_run.py --cell ice-twisted-w62-dimer1500 --pairs 1500 --weak 62 \
        --gpu-sweeps 131072,524288,1048576,2097152 --cluster-sweeps 131072,262144

Writes ``work/cells/<cell>/summary-<tag>.json`` with the QPU wins per rung for both solvers.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import threading
import time
from pathlib import Path
from typing import Any

import numpy as np

import window  # pyright: ignore[reportMissingImports]
from generate2 import SUITE_CELLS, add_dimers, add_loops, rescale_weak  # pyright: ignore[reportMissingImports]

ROOT = Path(__file__).resolve().parent
ADVERSARY = ROOT / "adversary"
PY = "/home/carback1/quip-miner-dwave/.venv/bin/python"
CASCADE_PY = "/home/carback1/quip-miner-dwave-cascade/.venv/bin/python"
HARNESS = "/home/carback1/.cache/round4-harness-target/release/round4-cuda-harness"
WORKER_DIR = "/home/carback1/quip-data/regimes/qpu-f17-worker"
SPEC = "/home/carback1/quip-data/regimes/topology-live-2026-09-21.spec.json"
QPU_CAP_S = 56.0
CALL_S = 0.046
N_SPINS = 4575


def energy(spins: np.ndarray, model: dict[str, np.ndarray]) -> np.ndarray:
    s = spins.astype(np.int64)
    edges = model["edges"]
    return (s[:, edges[:, 0]] * s[:, edges[:, 1]]) @ model["j_milli"] + s @ model["h_milli"]


def set_pair_strength(model: dict, milli: int) -> dict:
    j = model["j_milli"].copy()
    index = {(int(a), int(b)): i for i, (a, b) in enumerate(model["edges"].tolist())}
    for u, v in model.get("loop_edges", model["pairs"]).tolist():
        j[index[(min(u, v), max(u, v))]] = milli
    return {**model, "j_milli": j}


def build(args: argparse.Namespace, nonces: list[str], cell_dir: Path, tag: str) -> None:
    (cell_dir / "models").mkdir(parents=True, exist_ok=True)
    (cell_dir / "gpu").mkdir(exist_ok=True)
    lines, rows = [], []
    for nonce in nonces:
        path = cell_dir / "models" / f"{nonce}.npz"
        if not path.exists():
            with np.load(SUITE_CELLS / "ice-twisted" / "models" / f"{nonce}.npz") as data:
                model = {k: data[k] for k in data.files}
            if args.loops:
                model = add_loops(model, nonce, args.loops, args.loop_length)
            elif args.pairs:
                model = add_dimers(model, nonce, args.pairs)
            else:
                model = {**model, "pairs": np.zeros((0, 2), dtype=np.int64)}
            if args.pair_milli != -1000:
                model = set_pair_strength(model, args.pair_milli)
            model = rescale_weak(model, args.weak)
            keys = ("edges", "j_milli", "h_milli", "physical_nodes", "tetrahedra", "pairs", "loop_edges")
            keep = {k: model[k] for k in keys if k in model}
            np.savez_compressed(path, allow_pickle=False, **keep)
        with np.load(path) as data:
            sub = {"edges": data["edges"], "j_milli": data["j_milli"], "kept": np.arange(N_SPINS)}
            rows.append({"nonce": nonce, "pairs": int(len(data["pairs"]))})
        lines.extend(window.job_line(args.cell, nonce, sub, s) for s in args.gpu_sweeps)
    (cell_dir / "gpu" / f"jobs-{args.gpu_tag}.jsonl").write_text("\n".join(lines) + "\n")
    if tag == "screen":
        (cell_dir / "nonces.json").write_text(json.dumps(nonces, indent=1) + "\n")
        (cell_dir / "manifest.json").write_text(json.dumps(
            {"cell": args.cell, "base": "ice-twisted", "weak": args.weak, "pairs": args.pairs,
             "pair_milli": args.pair_milli, "models": rows}, indent=1) + "\n")


def capture(args: argparse.Namespace, nonces: list[str], tag: str) -> None:
    out = ROOT / "qpu" / args.cell / f"qpu-{args.anneal_us}"
    missing = [n for n in nonces if not (out / f"{n}.npz").exists()]
    if not missing:
        print("capture: present", flush=True)
        return
    spend = json.loads((ROOT / "qpu" / "spend-summary.json").read_text())["spent_s"]
    # The worker's own model: 35 ms of programming plus reads x (anneal + 150 us readout); 0.046 s at 48 x 80 us.
    estimate = len(missing) * (0.035 + args.reads * (args.anneal_us + 150) * 1e-6)
    print(f"capture: {len(missing)} models, estimate {estimate:.2f} s, spend {spend:.2f} -> {spend + estimate:.2f} of {QPU_CAP_S}", flush=True)
    if spend + estimate > QPU_CAP_S:
        raise SystemExit("capture would exceed the QPU cap")
    name = args.cell if tag == "screen" else f"{args.cell}-{tag}"
    (ROOT / "queue" / "requests" / f"{name}-{args.anneal_us}.json").write_text(json.dumps(
        {"cell": args.cell, "models_dir": f"work/cells/{args.cell}/models", "nonces": missing,
         "reads": args.reads, "anneal_us": args.anneal_us}) + "\n")
    stop = ROOT / "queue" / "STOP"
    stop.unlink(missing_ok=True)
    worker = subprocess.Popen([PY, "capture_worker_f17.py", "--suite-dir", str(ROOT), "--worker-dir", WORKER_DIR,
                               "--spec", SPEC, "--max-qpu-seconds", str(QPU_CAP_S), "--max-hours", "1",
                               "--poll-seconds", "10"], cwd=ROOT)
    deadline = time.time() + 3600
    while time.time() < deadline and worker.poll() is None:
        if all((out / f"{n}.npz").exists() for n in missing):
            break
        time.sleep(15)
    stop.touch()
    worker.wait(timeout=120)
    still = [n for n in missing if not (out / f"{n}.npz").exists()]
    if still:
        raise SystemExit(f"capture incomplete: {len(still)} models missing")
    print("capture: done", flush=True)


def gpu_ladder(args: argparse.Namespace, cell_dir: Path) -> None:
    out = cell_dir / "gpu" / f"n12-{args.gpu_tag}"
    jobs = cell_dir / "gpu" / f"jobs-{args.gpu_tag}.jsonl"
    wanted = len(jobs.read_text().splitlines())
    results = out / "results.jsonl"
    if results.exists() and len(results.read_text().splitlines()) >= wanted:
        print("gpu: present", flush=True)
        return
    subprocess.run([HARNESS, "0", str(jobs), str(out), "24"], cwd=ROOT, check=True)
    print("gpu: done", flush=True)


def cluster_ladder(args: argparse.Namespace, nonces: list[str], key: str) -> None:
    targets = json.loads((ADVERSARY / "targets.json").read_text())
    rows = []
    for nonce in nonces:
        model = ROOT / "work" / "cells" / args.cell / "models" / f"{nonce}.npz"
        with np.load(model) as md, np.load(ROOT / "qpu" / args.cell / "qpu-80" / f"{nonce}.npz") as qd:
            best = int(energy(qd["spins"], {k: md[k] for k in ("edges", "j_milli", "h_milli")}).min())
        rows.append({"nonce": nonce, "model": str(model), "qpu80_best": best})
    targets[key] = rows
    (ADVERSARY / "targets.json").write_text(json.dumps(targets, indent=1) + "\n")
    env = {**os.environ, "TMPDIR": str(ADVERSARY / "tmp"), "OPENBLAS_NUM_THREADS": "1", "OMP_NUM_THREADS": "1",
           "PYTHONDONTWRITEBYTECODE": "1"}
    subprocess.run([CASCADE_PY, "-B", "src/campaign.py", "prepare"], cwd=ADVERSARY, env=env, check=True)
    for sweeps in args.cluster_sweeps:
        for offset in args.seed_offsets:
            label = f"r2-{key}-s{sweeps}" + (f"-o{offset}" if offset else "")
            mdir = ADVERSARY / "measurements" / label
            if mdir.exists() and sum(1 for _ in mdir.glob("*.best.npy")) >= len(nonces):
                print(f"cluster {label}: present", flush=True)
                continue
            cmd = [CASCADE_PY, "-B", "src/round2.py", "--cell", key, "--label", label, "--workers", str(args.workers),
                   "--timeout", "7200", "--pair-mode", "soft", "--sweeps", str(sweeps), "--replicas", "8",
                   "--beta-min", "0.5", "--beta-max", "12.0", "--polish-sweeps", "32", "--dimer-weight-milli", "250",
                   "--sw-fraction", "0.0", "--schedule", "anneal", "--seed-offset", str(offset)]
            print("==", " ".join(cmd), flush=True)
            subprocess.run(cmd, cwd=ADVERSARY, env=env, check=False)
    print("cluster: done", flush=True)


def score(args: argparse.Namespace, nonces: list[str], cell_dir: Path, key: str, tag: str) -> None:
    models = {}
    for nonce in nonces:
        with np.load(cell_dir / "models" / f"{nonce}.npz") as data:
            models[nonce] = {k: data[k] for k in ("edges", "j_milli", "h_milli")}
    qpu = {}
    for nonce in nonces:
        with np.load(ROOT / "qpu" / args.cell / f"qpu-{args.anneal_us}" / f"{nonce}.npz") as data:
            qpu[nonce] = int(energy(data["spins"], models[nonce]).min())
    summary: dict[str, Any] = {"cell": args.cell, "tag": tag, "anneal_us": args.anneal_us, "gpu": {}, "cluster": {}}
    out = cell_dir / "gpu" / f"n12-{args.gpu_tag}"
    kernel = {}
    results = out / "results.jsonl"
    for line in results.read_text().splitlines() if results.exists() else []:
        row = json.loads(line)
        if row["ok"]:
            kernel[row["id"]] = row["kernel_s"]
    for sweeps in args.gpu_sweeps:
        gaps, secs = [], []
        for nonce in nonces:
            job = f"{args.cell}-{nonce[:16]}-s{sweeps}-r64-msa-i32"
            if job not in kernel:
                continue
            spins = np.fromfile(out / "spins" / f"{job}.bin", dtype=np.int8).reshape(-1, N_SPINS)
            gaps.append(int(energy(spins, models[nonce]).min()) - qpu[nonce])
            secs.append(kernel[job])
        if gaps:
            summary["gpu"][str(sweeps)] = {"n": len(gaps), "qpu_wins": sum(g > 0 for g in gaps), "ties": sum(g == 0 for g in gaps),
                                           "median_gap": int(np.median(gaps)), "median_s": float(np.median(secs))}
    for sweeps in args.cluster_sweeps:
        for offset in args.seed_offsets:
            label = f"r2-{key}-s{sweeps}" + (f"-o{offset}" if offset else "")
            mdir = ADVERSARY / "measurements" / label
            gaps, secs, mismatch = [], [], 0
            for nonce in nonces:
                path, best = mdir / f"{nonce}.jsonl", mdir / f"{nonce}.best.npy"
                if not (path.exists() and best.exists()):
                    continue
                record = json.loads(path.read_text().splitlines()[-1])
                value = int(energy(np.load(best)[None, :], models[nonce])[0])
                mismatch += value != record["best_milli"]
                gaps.append(value - qpu[nonce])
                secs.append(record["seconds"])
            if gaps:
                summary["cluster"][label] = {"n": len(gaps), "qpu_wins": sum(g > 0 for g in gaps), "ties": sum(g == 0 for g in gaps),
                                             "median_gap": int(np.median(gaps)), "median_s": float(np.median(secs)),
                                             "rescore_mismatches": mismatch}
    (cell_dir / f"summary-{tag}.json").write_text(json.dumps(summary, indent=1) + "\n")
    print(json.dumps(summary, indent=1), flush=True)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cell", required=True)
    parser.add_argument("--pairs", type=int, default=0)
    parser.add_argument("--loops", type=int, default=0, help="closed loops of weak couplers at --pair-milli instead of pairs")
    parser.add_argument("--loop-length", type=int, default=4)
    parser.add_argument("--weak", type=int, default=62)
    parser.add_argument("--pair-milli", type=int, default=-1000)
    parser.add_argument("--nonces", default="0:12", help="slice of the suite's ice-twisted nonces")
    parser.add_argument("--tag", default=None, help="cohort tag; 'screen' for the first 12 nonces, else 'ext'")
    parser.add_argument("--gpu-tag", default=None)
    parser.add_argument("--gpu-sweeps", default="131072,524288,1048576,2097152")
    parser.add_argument("--cluster-sweeps", default="131072,262144")
    parser.add_argument("--seed-offsets", default="0")
    parser.add_argument("--workers", type=int, default=12)
    parser.add_argument("--reads", type=int, default=48)
    parser.add_argument("--anneal-us", type=int, default=80)
    parser.add_argument("--skip-cluster", action="store_true")
    parser.add_argument("--skip-gpu", action="store_true")
    args = parser.parse_args()
    args.gpu_sweeps = [int(s) for s in args.gpu_sweeps.split(",") if s]
    args.cluster_sweeps = [int(s) for s in args.cluster_sweeps.split(",") if s]
    args.seed_offsets = [int(s) for s in args.seed_offsets.split(",")]
    start, stop = (int(x) for x in args.nonces.split(":"))
    tag = args.tag or ("screen" if start == 0 else "ext")
    args.gpu_tag = args.gpu_tag or tag
    key = args.cell if tag == "screen" else f"{args.cell}-{tag}"
    nonces = json.loads((SUITE_CELLS / "ice-twisted" / "nonces.json").read_text())[start:stop]
    cell_dir = ROOT / "work" / "cells" / args.cell
    build(args, nonces, cell_dir, tag)
    print(f"{args.cell}: {len(nonces)} models, tag {tag}, key {key}", flush=True)
    capture(args, nonces, tag)
    threads = []
    if not args.skip_gpu:
        threads.append(threading.Thread(target=gpu_ladder, args=(args, cell_dir)))
    if not args.skip_cluster and args.anneal_us == 80:
        threads.append(threading.Thread(target=cluster_ladder, args=(args, nonces, key)))
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    score(args, nonces, cell_dir, key, tag)
    print("wave done", flush=True)


if __name__ == "__main__":
    main()
