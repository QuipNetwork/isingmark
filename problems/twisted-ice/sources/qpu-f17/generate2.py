#!/usr/bin/env python3
"""Variant cells derived from the suite's ice models, for the GPU-hardness search.

Each variant starts from the suite's saved models of ``ice-twisted`` or ``ice-glass-r8``
(same 12 nonces, same tetrahedra) and changes one ingredient:

- ``ice-twisted-w62``: weak layer at 62 milli (ratio 16) instead of 125.
- ``ice-twisted-w250``: weak layer at 250 milli (ratio 4).
- ``ice-twisted-dimer``: 600 extra ferromagnetic pairs at -1,000 on weak couplers, a greedy
  nonce-seeded matching that avoids tetrahedron bonds.
- ``ice-random-k4``: every tetrahedron bond gets an independent random sign at 1,000 instead of
  the gauge pattern (no ice rule, a random K4 glass network).
- ``ice-glass-r16``: untwisted ice with the weak layer at 62 milli.

Writes work/cells/<cell>/models/<nonce>.npz, nonces.json, manifest.json, a GPU job file at
8,192 / 32,768 / 131,072 sweeps, and a production capture request.
"""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

import numpy as np

import window  # pyright: ignore[reportMissingImports]

ROOT = Path(__file__).resolve().parent
SUITE_CELLS = Path("/home/carback1/quip-data/regimes/qpu-suite/work/cells")
SWEEPS = (8192, 32768, 131072)
MODELS = 12


def rng_for(nonce: str, tag: str) -> np.random.Generator:
    digest = hashlib.sha256(tag.encode() + bytes.fromhex(nonce)).digest()
    return np.random.default_rng(int.from_bytes(digest[:8], "little"))


def rescale_weak(model: dict, to: int) -> dict:
    j = model["j_milli"].copy()
    weak = np.abs(j) == 125
    j[weak] = np.sign(j[weak]) * to
    return {**model, "j_milli": j}


def add_dimers(model: dict, nonce: str, count: int) -> dict:
    edges, j = model["edges"], model["j_milli"].copy()
    tetra_spin = np.zeros(4575, dtype=bool)
    tetra_spin[model["tetrahedra"].ravel()] = True
    weak = np.flatnonzero(np.abs(j) == 125)
    order = rng_for(nonce, "dimer").permutation(weak)
    used = np.zeros(4575, dtype=bool)
    pairs = []
    for index in order:
        u, v = edges[index]
        if used[u] or used[v]:
            continue
        used[u] = used[v] = True
        j[index] = -1000
        pairs.append((int(u), int(v)))
        if len(pairs) == count:
            break
    return {**model, "j_milli": j, "pairs": np.asarray(pairs, dtype=np.int64)}


def find_loop(start: int, length: int, adj: list[list[int]], used: np.ndarray, rng: np.random.Generator) -> list[int] | None:
    """A simple cycle of ``length`` weak couplers through ``start`` over unused spins, or None."""
    path = [start]
    on_path = {start}
    budget = 2000

    def extend() -> bool:
        nonlocal budget
        budget -= 1
        if budget < 0:
            return False
        u = path[-1]
        if len(path) == length:
            return start in adj[u]
        for v in rng.permutation(adj[u]).tolist():
            if used[v] or v in on_path:
                continue
            path.append(v)
            on_path.add(v)
            if extend():
                return True
            path.pop()
            on_path.discard(v)
        return False

    return path if extend() else None


def add_loops(model: dict, nonce: str, count: int, length: int) -> dict:
    """Plant ``count`` spin-disjoint closed loops of ``length`` weak couplers at -1000.

    ``loop_edges`` holds every planted edge. ``pairs`` holds every other edge of each loop, a
    matching, because the cluster solver contracts pairs and rejects overlapping ones.
    """
    edges, j = model["edges"], model["j_milli"].copy()
    adj: list[list[int]] = [[] for _ in range(4575)]
    index = {}
    for i in np.flatnonzero(np.abs(j) == 125).tolist():
        u, v = int(edges[i, 0]), int(edges[i, 1])
        adj[u].append(v)
        adj[v].append(u)
        index[(min(u, v), max(u, v))] = i
    rng = rng_for(nonce, "loop")
    used = np.zeros(4575, dtype=bool)
    loops = []
    for start in rng.permutation(4575).tolist():
        if used[start]:
            continue
        loop = find_loop(start, length, adj, used, rng)
        if loop is None:
            continue
        used[loop] = True
        loops.append(loop)
        if len(loops) == count:
            break
    if len(loops) < count:
        raise ValueError(f"only {len(loops)} loops of {length} fit, wanted {count}")
    loop_edges, pairs = [], []
    for loop in loops:
        for k in range(length):
            u, v = loop[k], loop[(k + 1) % length]
            j[index[(min(u, v), max(u, v))]] = -1000
            loop_edges.append((u, v))
            if k % 2 == 0 and k + 1 < length:
                pairs.append((u, v))
    return {**model, "j_milli": j, "pairs": np.asarray(pairs, dtype=np.int64),
            "loop_edges": np.asarray(loop_edges, dtype=np.int64)}


def random_k4(model: dict, nonce: str) -> dict:
    edges, j = model["edges"], model["j_milli"].copy()
    index = {(int(a), int(b)): i for i, (a, b) in enumerate(edges.tolist())}
    rng = rng_for(nonce, "random-k4")
    for t in model["tetrahedra"]:
        for a in range(4):
            for b in range(a + 1, 4):
                key = (min(int(t[a]), int(t[b])), max(int(t[a]), int(t[b])))
                j[index[key]] = 1000 * int(rng.choice([-1, 1]))
    return {**model, "j_milli": j}


VARIANTS = {
    "ice-twisted-w62": ("ice-twisted", lambda m, _: rescale_weak(m, 62)),
    "ice-twisted-w250": ("ice-twisted", lambda m, _: rescale_weak(m, 250)),
    "ice-twisted-dimer": ("ice-twisted", lambda m, n: add_dimers(m, n, 600)),
    "ice-random-k4": ("ice-twisted", random_k4),
    "ice-glass-r16": ("ice-glass-r8", lambda m, _: rescale_weak(m, 62)),
    # Wave 2: push the two ingredients that helped against the GPU in wave 1.
    "ice-twisted-w31": ("ice-twisted", lambda m, _: rescale_weak(m, 31)),
    "ice-twisted-w62-dimer": ("ice-twisted", lambda m, n: rescale_weak(add_dimers(m, n, 600), 62)),
    "ice-twisted-dimer1200": ("ice-twisted", lambda m, n: add_dimers(m, n, 1200)),
    # Wave 4: locate the weak-scale optimum between 31 (too close to the noise floor) and 125.
    "ice-twisted-w45": ("ice-twisted", lambda m, _: rescale_weak(m, 45)),
    "ice-twisted-w90": ("ice-twisted", lambda m, _: rescale_weak(m, 90)),
    # Wave 6: the adversary's cluster solver closes the 62 cells in seconds and the 125 cells in
    # minutes, while single flips rank them the other way. Probe the band between, with pairs.
    "ice-twisted-w90-dimer": ("ice-twisted", lambda m, n: rescale_weak(add_dimers(m, n, 600), 90)),
    "ice-twisted-w62-dimer1200": ("ice-twisted", lambda m, n: rescale_weak(add_dimers(m, n, 1200), 62)),
}


def main(cells: list[str]) -> None:
    for cell in cells:
        base, transform = VARIANTS[cell]
        nonces = json.loads((SUITE_CELLS / base / "nonces.json").read_text())[:MODELS]
        cell_dir = ROOT / "work" / "cells" / cell
        (cell_dir / "models").mkdir(parents=True, exist_ok=True)
        (cell_dir / "gpu").mkdir(exist_ok=True)
        lines = []
        rows = []
        for nonce in nonces:
            with np.load(SUITE_CELLS / base / "models" / f"{nonce}.npz") as data:
                model = {k: data[k] for k in data.files}
            out = transform(model, nonce)
            keep = {k: out[k] for k in ("edges", "j_milli", "h_milli", "physical_nodes", "tetrahedra")}
            if "pairs" in out:
                keep["pairs"] = out["pairs"]
            np.savez_compressed(cell_dir / "models" / f"{nonce}.npz", allow_pickle=False, **keep)
            sub = {"edges": keep["edges"], "j_milli": keep["j_milli"], "kept": np.arange(4575)}
            lines.extend(window.job_line(cell, nonce, sub, s) for s in SWEEPS)
            magnitudes = np.unique(np.abs(keep["j_milli"]))
            rows.append({"nonce": nonce, "edges": int(len(keep["edges"])), "magnitudes": magnitudes.tolist(),
                         "pairs": int(len(out["pairs"])) if "pairs" in out else 0})
        (cell_dir / "nonces.json").write_text(json.dumps(nonces, indent=1) + "\n")
        (cell_dir / "manifest.json").write_text(json.dumps({"cell": cell, "base": base, "models": rows}, indent=1) + "\n")
        (cell_dir / "gpu" / "jobs-n12-screen.jsonl").write_text("\n".join(lines) + "\n")
        (ROOT / "queue" / "requests" / f"{cell}-80.json").write_text(json.dumps(
            {"cell": cell, "models_dir": f"work/cells/{cell}/models", "nonces": nonces, "reads": 48, "anneal_us": 80}) + "\n")
        print(cell, rows[0], flush=True)


if __name__ == "__main__":
    main(sys.argv[1:] or list(VARIANTS))
