#!/usr/bin/env python3
"""Wave 3: overlapping and charged ice on all 6,822 K4s of the live graph.

Constructions from the research report (`research/report.md`, rows 1, 2 and 6):

- ``ice-overlap2``: K4s accepted in nonce order while every coupler is used by at most two
  tiles and every spin by at most three. Tile scale A = 500 with an independent gauge per
  tile, contributions summed on shared couplers, background ±31 on unused couplers.
- ``ice-all-k4``: every K4 of the graph, A = 250, independent gauges, summed, background ±31.
- ``ice-charge``: the suite's edge-disjoint tetrahedra at A = 250 with a prescribed charge
  q in {0, +2, -2} drawn with probabilities 1/2, 1/4, 1/4 per tile, realized by the squared
  charge penalty (A/2)(sum g s - q)^2: J = A g g, h = -A q g summed per spin. Background ±31.

Every cell writes 12 models in the suite schema plus a GPU screen job file and a production
capture request.
"""

from __future__ import annotations

import hashlib
import json
import sys
from itertools import combinations
from pathlib import Path

import numpy as np

import window  # pyright: ignore[reportMissingImports]
from quip_miner_dwave.round2_runner import logical_beta_range  # pyright: ignore[reportMissingImports]

ROOT = Path(__file__).resolve().parent
SPEC = Path("/home/carback1/quip-data/regimes/topology-live-2026-09-21.spec.json")
SUITE_TWISTED = Path("/home/carback1/quip-data/regimes/qpu-suite/work/cells/ice-twisted")
SWEEPS = (8192, 32768, 131072)
N = 4575


def load_graph() -> tuple[np.ndarray, np.ndarray]:
    spec = json.loads(SPEC.read_text())
    nodes = np.asarray(spec["nodes"], dtype=np.int64)
    index = {int(q): i for i, q in enumerate(nodes)}
    edges = np.asarray([(index[u], index[v]) for u, v in spec["edges"]], dtype=np.int64)
    edges = np.sort(edges, axis=1)
    return nodes, edges


def enumerate_k4(edges: np.ndarray) -> np.ndarray:
    neighbours: list[set[int]] = [set() for _ in range(N)]
    for u, v in edges.tolist():
        neighbours[u].add(v)
        neighbours[v].add(u)
    found = set()
    for u, v in edges.tolist():
        common = sorted(neighbours[u] & neighbours[v])
        for a, b in combinations(common, 2):
            if b in neighbours[a]:
                found.add(tuple(sorted((u, v, a, b))))
    return np.asarray(sorted(found), dtype=np.int64)


def rng_for(nonce: str, tag: str) -> np.random.Generator:
    digest = hashlib.sha256(tag.encode() + bytes.fromhex(nonce)).digest()
    return np.random.default_rng(int.from_bytes(digest[:8], "little"))


def build(edges: np.ndarray, edge_index: dict, tiles: np.ndarray, gauges: np.ndarray, charges: np.ndarray,
          scale: int, background: int, rng: np.random.Generator) -> tuple[np.ndarray, np.ndarray]:
    j = np.zeros(len(edges), dtype=np.int64)
    h = np.zeros(N, dtype=np.int64)
    used = np.zeros(len(edges), dtype=bool)
    for tile, gauge, q in zip(tiles, gauges, charges):
        for a, b in combinations(range(4), 2):
            k = edge_index[(min(int(tile[a]), int(tile[b])), max(int(tile[a]), int(tile[b])))]
            j[k] += scale * int(gauge[a]) * int(gauge[b])
            used[k] = True
        if q:
            for a in range(4):
                h[int(tile[a])] += -scale * int(q) * int(gauge[a])
    free = np.flatnonzero(~used)
    j[free] = rng.choice(np.asarray([-background, background]), size=len(free))
    if np.abs(j).max() > 1000 or np.abs(h).max() > 1000:
        raise ValueError("coefficient out of range")
    return j, h


def select_overlap(k4: np.ndarray, edge_index: dict, rng: np.random.Generator, edge_cap: int, spin_cap: int) -> np.ndarray:
    order = rng.permutation(len(k4))
    edge_use = np.zeros(len(edge_index), dtype=np.int64)
    spin_use = np.zeros(N, dtype=np.int64)
    chosen = []
    for t in k4[order]:
        keys = [edge_index[(min(int(t[a]), int(t[b])), max(int(t[a]), int(t[b])))] for a, b in combinations(range(4), 2)]
        if edge_use[keys].max() >= edge_cap or spin_use[t].max() >= spin_cap:
            continue
        edge_use[keys] += 1
        spin_use[t] += 1
        chosen.append(t)
    return np.asarray(chosen, dtype=np.int64)


def main(cells: list[str]) -> None:
    nodes, edges = load_graph()
    edge_index = {(int(a), int(b)): i for i, (a, b) in enumerate(edges.tolist())}
    k4 = enumerate_k4(edges)
    print("K4 count", len(k4), flush=True)
    nonces = json.loads((SUITE_TWISTED / "nonces.json").read_text())[:12]
    for cell in cells:
        cell_dir = ROOT / "work" / "cells" / cell
        (cell_dir / "models").mkdir(parents=True, exist_ok=True)
        (cell_dir / "gpu").mkdir(exist_ok=True)
        lines, rows = [], []
        for nonce in nonces:
            rng = rng_for(nonce, cell)
            if cell == "ice-overlap2":
                tiles = select_overlap(k4, edge_index, rng, edge_cap=2, spin_cap=3)
                scale, background = 500, 31
                charges = np.zeros(len(tiles), dtype=np.int64)
            elif cell == "ice-all-k4":
                tiles = k4
                scale, background = 250, 31
                charges = np.zeros(len(tiles), dtype=np.int64)
            elif cell == "ice-charge":
                with np.load(SUITE_TWISTED / "models" / f"{nonce}.npz") as data:
                    tiles = data["tetrahedra"]
                scale, background = 250, 31
                charges = rng.choice(np.asarray([0, 2, -2]), size=len(tiles), p=[0.5, 0.25, 0.25])
            else:
                raise ValueError(cell)
            gauges = rng.choice(np.asarray([-1, 1], dtype=np.int64), size=(len(tiles), 4))
            j, h = build(edges, edge_index, tiles, gauges, charges, scale, background, rng)
            keep = j != 0
            model = {"edges": edges[keep], "j_milli": j[keep], "h_milli": h, "physical_nodes": nodes,
                     "tetrahedra": tiles, "local_gauges": gauges.astype(np.int8), "charges": charges}
            np.savez_compressed(cell_dir / "models" / f"{nonce}.npz", allow_pickle=False, **model)
            # window.job_line zeroes the fields; ice-charge carries fields, so build the job here.
            h = model["h_milli"] / 1000.0
            j = model["j_milli"] / 1000.0
            beta = logical_beta_range(h, model["edges"], j)
            for s in SWEEPS:
                lines.append(json.dumps({
                    "id": f"{cell}-{nonce[:16]}-s{s}-r64-msa-i32", "cell": cell, "nonce": nonce, "h": h.tolist(),
                    "edges": model["edges"].tolist(), "j": j.tolist(), "beta": [float(beta[0]), float(beta[1])],
                    "seed": window.protocol_seed(nonce), "sweeps": s, "reads": 64, "kernel": "msa", "lane": "i32",
                }, separators=(",", ":")))
            incidence = np.bincount(tiles.ravel(), minlength=N)
            rows.append({"nonce": nonce, "tiles": int(len(tiles)), "edges": int(keep.sum()),
                         "max_abs_j": int(np.abs(j).max()), "fields_nonzero": int((h != 0).sum()),
                         "max_incidence": int(incidence.max()), "cancelled": int(((~keep) & (j == 0)).sum())})
        (cell_dir / "nonces.json").write_text(json.dumps(nonces, indent=1) + "\n")
        (cell_dir / "manifest.json").write_text(json.dumps({"cell": cell, "k4_total": int(len(k4)), "models": rows},
                                                            indent=1) + "\n")
        (cell_dir / "gpu" / "jobs-n12-screen.jsonl").write_text("\n".join(lines) + "\n")
        (ROOT / "queue" / "requests" / f"{cell}-80.json").write_text(json.dumps(
            {"cell": cell, "models_dir": f"work/cells/{cell}/models", "nonces": nonces, "reads": 48, "anneal_us": 80}) + "\n")
        print(cell, rows[0], flush=True)


if __name__ == "__main__":
    main(sys.argv[1:] or ["ice-overlap2", "ice-all-k4", "ice-charge"])
