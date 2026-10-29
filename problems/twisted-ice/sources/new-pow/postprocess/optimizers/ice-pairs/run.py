#!/usr/bin/env python3
"""Postprocessing optimizer for twisted ice with planted pairs (family ice-pairs).

Usage: run.py --cell CELL --budget-s S --out DIR [--models 12|all] [--threads N] [--method M] [--force]

Per model the 48 production reads are descended within S seconds of CPU on one core. The schedule
visits the reads from the lowest raw energy up: natural-move descent, then the pair-aware worm of
pairworm.c alternated with the descent until the energy stops falling. Every read that finished in
time joins a pool; the best pool state then takes every improving strong-connected block from the
other pool states (crossover), and the worm runs once more on the result. The move tables and the
tile, pair and strong-graph lists are model-only setup, timed apart as setup_cpu_s.

Writes DIR/results.jsonl (one row per model) and DIR/states/<nonce>.npy (int8, chip index).
"""
from __future__ import annotations

import argparse
import json
import time
from pathlib import Path

import numpy as np

import descent as D
import pairs as P

TARGETS = D.NP_DIR / "targets-greedy.json"


class Context:
    """Model-only preparation: natural moves, compiled descent, tiles and pairs."""

    def __init__(self, model: dict[str, np.ndarray]):
        self.model = model
        self.ising = D.Ising(model)
        moves, _ = D.natural_moves(self.ising)
        self.gnat = D.GreedyC(self.ising, D.MoveSet(self.ising, moves))
        self.loop = D.LoopC(self.ising, self.gnat)
        self.pw = P.PairWorm(self.ising, self.gnat)

    def energies(self, s: np.ndarray) -> np.ndarray:
        return self.ising.energies(s)


def alternate(ctx: Context, s: np.ndarray, worm, rounds: int = 10) -> np.ndarray:
    """Natural-move descent, then worm and descent in turn until no read's energy falls."""
    s, _ = ctx.gnat.descend(s)
    e = ctx.energies(s)
    for _ in range(rounds):
        s = worm(s)
        s, _ = ctx.gnat.descend(s)
        e_new = ctx.energies(s)
        if np.array_equal(e_new, e):
            break
        e = e_new
    return s


class Budget:
    def __init__(self, seconds: float):
        self.limit = seconds
        self.t0 = time.process_time()

    def used(self) -> float:
        return time.process_time() - self.t0

    def left(self) -> float:
        return self.limit - self.used()


def schedule_pw(ctx: Context, reads: np.ndarray, budget: Budget, seed: int, opts: dict) -> tuple[np.ndarray, dict]:
    """Reads by raw energy through the pair-aware worm, then crossover rounds into the best state, then
    anneal-quench cycles of the worm on the best state (Metropolis sweeps at `temp` milli, strict
    descent, keep the best quenched state) for the tail of the budget. With `anneal_share` 0 the
    tail goes to more reads and a zero-budget worm polish. `pre_reads` descends that many of the
    lowest raw reads with natural moves first and visits them by descended energy."""
    n = ctx.ising.n
    max_len = int(opts.get("max_len", 32))
    depth = int(opts.get("defect_depth", 8))
    reserve = float(opts.get("reserve", min(0.02, 0.1 * budget.limit)))
    temp = float(opts.get("temp", 250.0))
    cycle_sweeps = int(opts.get("cycle_sweeps", 1))
    pre_reads = int(opts.get("pre_reads", 0))
    if "anneal_share" in opts:
        share = float(opts["anneal_share"])
        read_limit = budget.limit * (1.0 - share)
    else:
        # reads keep the first 0.05 s plus 45 percent of the rest; the anneal tail takes what remains
        read_limit = min(budget.limit, 0.05 + 0.45 * budget.limit)
        share = 1.0 - read_limit / budget.limit

    def strict(x: np.ndarray, k: int) -> np.ndarray:
        return ctx.pw.descend(x, 0, seed + k, max_len=max_len, defect_depth=depth)[0]

    order = np.argsort(ctx.energies(reads), kind="stable")
    if pre_reads > 0:
        # natural-move descent on the lowest raw reads first, then visit them by descended energy
        head = order[:pre_reads]
        reads = reads.copy()
        reads[head], _ = ctx.gnat.descend(reads[head])
        order = np.concatenate([head[np.argsort(ctx.energies(reads[head]), kind="stable")], order[pre_reads:]])
    pool: list[np.ndarray] = []
    per_read: list[float] = []
    for k, r in enumerate(order):
        est = max(per_read) if per_read else 0.0
        if k > 0 and budget.used() + est + reserve > read_limit:
            break
        t0 = budget.used()
        s = alternate(ctx, reads[r:r + 1], lambda x: strict(x, k))
        pool.append(s[0])
        per_read.append(budget.used() - t0)
    states = np.stack(pool)
    e = ctx.energies(states)
    best = int(e.argmin())
    info = {"reads_used": len(pool), "read_cpu_s": per_read, "crossover_blocks": 0, "crossover_rounds": 0,
            "pre_crossover": int(e[best]), "pre_anneal": int(e[best]), "anneal_cycles": 0, "polish": False}
    s = states[best]
    if len(pool) > 1 and opts.get("crossover", True):
        others = np.delete(states, best, axis=0)
        for rnd in range(int(opts.get("crossover_rounds", 3))):
            s_new, blocks = ctx.pw.crossover(s, others)
            if blocks == 0:
                break
            info["crossover_blocks"] += blocks
            info["crossover_rounds"] = rnd + 1
            s = alternate(ctx, s_new[None], lambda x: strict(x, 1000 + rnd))[0]
    eb = int(ctx.energies(s[None])[0])
    info["pre_anneal"] = eb
    if share > 0:
        cycle_cost = 0.0
        k = 0
        while budget.left() > cycle_cost + reserve:
            t0 = budget.used()
            a, _ = ctx.pw.descend(s[None], 0, seed + 3000 + k, max_sweeps=cycle_sweeps, max_len=max_len, temp=temp)
            q = alternate(ctx, a, lambda x: strict(x, 4000 + k))
            eq = int(ctx.energies(q)[0])
            if eq < eb:
                eb, s = eq, q[0]
            k += 1
            cycle_cost = max(cycle_cost, budget.used() - t0)
        info["anneal_cycles"] = k
    elif budget.left() > 1.5 * max(per_read) and opts.get("polish", True):
        info["polish"] = True
        s = alternate(ctx, s[None], lambda x: ctx.pw.descend(x, 4 * n, seed + 2000, max_len=max_len,
                                                             defect_depth=depth)[0])[0]
    return s, info


def schedule_loop(ctx: Context, reads: np.ndarray, budget: Budget, seed: int, opts: dict) -> tuple[np.ndarray, dict]:
    """Reference: the current postprocess (natural moves, strict worm of greedy.c) under the same schedule."""
    order = np.argsort(ctx.energies(reads), kind="stable")
    pool: list[np.ndarray] = []
    per_read: list[float] = []
    for k, r in enumerate(order):
        est = max(per_read) if per_read else 0.0
        if k > 0 and budget.left() < est:
            break
        t0 = budget.used()
        s = alternate(ctx, reads[r:r + 1], lambda x: ctx.loop.descend(x, 0, seed + k)[0])
        pool.append(s[0])
        per_read.append(budget.used() - t0)
    states = np.stack(pool)
    e = ctx.energies(states)
    return states[int(e.argmin())], {"reads_used": len(pool)}


SCHEDULES = {"pw": schedule_pw, "loop": schedule_loop}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cell", required=True)
    ap.add_argument("--budget-s", type=float, required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--models", default="12")
    ap.add_argument("--threads", type=int, default=1)
    ap.add_argument("--method", default="pw")
    ap.add_argument("--opts", default="{}", help="JSON options for the schedule")
    ap.add_argument("--force", action="store_true")
    a = ap.parse_args()
    if a.threads != 1:
        raise SystemExit("this optimizer runs on one thread")
    opts = json.loads(a.opts)
    out = Path(a.out)
    (out / "states").mkdir(parents=True, exist_ok=True)
    res = out / "results.jsonl"
    done = set()
    if res.exists() and not a.force:
        done = {json.loads(x)["nonce"] for x in res.read_text().splitlines() if x.strip()}
    elif res.exists():
        res.unlink()
    rows = json.loads(TARGETS.read_text())[a.cell]
    if a.models != "all":
        rows = rows[: int(a.models)]
    for row in rows:
        nonce = row["nonce"]
        if nonce in done:
            continue
        arch = D.archive(a.cell, nonce)
        if arch is None:
            raise FileNotFoundError(f"{a.cell} {nonce[:8]}: no archive")
        with np.load(arch) as z:
            reads = z["spins"].astype(np.int8)
        model = D.load_model(row["model"])
        t0 = time.process_time()
        ctx = Context(model)
        setup = time.process_time() - t0
        seed = int(nonce[:12], 16)
        w0 = time.perf_counter()
        budget = Budget(a.budget_s)
        s, info = SCHEDULES[a.method](ctx, reads, budget, seed, opts)
        cpu = budget.used()
        wall = time.perf_counter() - w0
        e = int(D.rescore.energy(s, model))
        if e != int(ctx.energies(s[None])[0]):
            raise RuntimeError(f"{a.cell} {nonce[:8]}: energy mismatch")
        np.save(out / "states" / f"{nonce}.npy", s.astype(np.int8))
        rec = {"cell": a.cell, "nonce": nonce, "qpu80_best": int(row["qpu80_best"]),
               "qpu80_greedy": int(row["qpu80_greedy"]), "budget_s": a.budget_s, "pp_best": e, "cpu_s": cpu,
               "wall_s": wall, "threads": 1, "setup_cpu_s": setup, "method": a.method, "opts": opts, "seed": seed,
               "tiles": ctx.pw.counts, **info}
        with res.open("a") as f:
            f.write(json.dumps(rec) + "\n")
        print(a.cell, nonce[:8], "gain", int(row["qpu80_greedy"]) - e, "cpu", round(cpu, 3), "reads", info["reads_used"],
              "xo", info.get("crossover_blocks", 0), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
