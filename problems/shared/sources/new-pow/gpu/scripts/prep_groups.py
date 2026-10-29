"""Build the gpupair input file from a saved integer model (.npz): rigid groups, two colorings, ELL tables."""
from __future__ import annotations

import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import prep  # noqa: E402

STRONG_MILLI = -500  # planted bonds are at or below this and sit outside every tetrahedron


def strong_edges(model: dict) -> np.ndarray:
    """Rows of `edges` that are planted rigid bonds: j <= -500 and not inside a tetrahedron."""
    edges = model["edges"].astype(np.int64)
    n = len(model["h_milli"])
    lo, hi = np.minimum(edges[:, 0], edges[:, 1]), np.maximum(edges[:, 0], edges[:, 1])
    keys = lo * n + hi
    strong = model["j_milli"] <= STRONG_MILLI
    if "tetrahedra" in model and len(model["tetrahedra"]):
        tet = model["tetrahedra"].astype(np.int64)
        tk = [np.minimum(tet[:, a], tet[:, b]) * n + np.maximum(tet[:, a], tet[:, b])
              for a in range(4) for b in range(a + 1, 4)]
        strong &= ~np.isin(keys, np.concatenate(tk))
    return edges[strong]


def extract_groups(model: dict) -> np.ndarray:
    """Group id per spin (ids by first member), from the strong edges and `pairs` when present."""
    n = len(model["h_milli"])
    parts = [strong_edges(model)]
    if "pairs" in model and len(model["pairs"]):
        parts.append(model["pairs"].astype(np.int64))
    parent = list(range(n))

    def find(x: int) -> int:
        while parent[x] != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x

    for u, v in np.concatenate(parts).tolist():
        ru, rv = find(u), find(v)
        if ru != rv:
            parent[max(ru, rv)] = min(ru, rv)
    roots = np.array([find(i) for i in range(n)])
    _, gid = np.unique(roots, return_inverse=True)
    return gid.astype(np.int64)


def group_stats(gid: np.ndarray) -> dict:
    sizes = np.bincount(gid)
    return {"sites": int(len(sizes)), "groups": int((sizes > 1).sum()), "largest_group": int(sizes.max()),
            "size_histogram": {int(k): int(v) for k, v in zip(*np.unique(sizes, return_counts=True), strict=True)}}


def contracted(model: dict, gid: np.ndarray) -> tuple[np.ndarray, np.ndarray, np.ndarray, int]:
    """Contracted Ising model: (h_g, edges_g, j_g, const). For states with aligned groups,
    E(s) = E_g(sigma) + const, with s = sigma[gid]. Parallel edges are merged, internal bonds are const."""
    g_count = int(gid.max()) + 1
    h_g = np.bincount(gid, weights=model["h_milli"], minlength=g_count).astype(np.int64)
    edges = model["edges"].astype(np.int64)
    j = model["j_milli"].astype(np.int64)
    gu, gv = gid[edges[:, 0]], gid[edges[:, 1]]
    inside = gu == gv
    const = int(j[inside].sum())
    a, b = np.minimum(gu, gv)[~inside], np.maximum(gu, gv)[~inside]
    keys, inv = np.unique(a * g_count + b, return_inverse=True)
    merged = np.zeros(len(keys), dtype=np.int64)
    np.add.at(merged, inv, j[~inside])
    return h_g, np.stack([keys // g_count, keys % g_count], axis=1), merged, const


def contracted_energy(h_g: np.ndarray, edges_g: np.ndarray, j_g: np.ndarray, sigma: np.ndarray) -> int:
    s = sigma.astype(np.int64)
    return int((s[edges_g[:, 0]] * s[edges_g[:, 1]]) @ j_g + s @ h_g)


def build(model: dict) -> tuple[bytes, np.ndarray, np.ndarray, dict]:
    """Returns (input blob, perm, gorder, stats). Row k of the device order is spin perm[k] of the model
    order, and device group k is group gorder[k] of `extract_groups`."""
    edges = model["edges"].astype(np.int64)
    j = model["j_milli"].astype(np.int64)
    h = model["h_milli"].astype(np.int64)
    n = len(h)
    if n > 65535 or np.abs(j).max() > 32767:
        raise ValueError("out of range")
    gid = extract_groups(model)
    stats = group_stats(gid)
    g_count = stats["sites"]

    spin_off, perm = prep.coloring(n, edges)
    new = np.empty(n, dtype=np.int64)
    new[perm] = np.arange(n)
    e2 = new[edges]
    all_deg, all_ell = prep.ell(n, e2[:, 0], e2[:, 1], j)

    ext = gid[edges[:, 0]] != gid[edges[:, 1]]
    ext_deg, ext_ell = prep.ell(n, e2[ext, 0], e2[ext, 1], j[ext])
    # Groups share a color only when no nonzero coupling joins any member of one to any member of the other.
    live = ext & (j != 0)
    ga, gb = gid[edges[live, 0]], gid[edges[live, 1]]
    pair_keys = np.unique(np.minimum(ga, gb) * g_count + np.maximum(ga, gb))
    group_edges = np.stack([pair_keys // g_count, pair_keys % g_count], axis=1)
    group_off, gorder = prep.coloring(g_count, group_edges)

    rank = np.empty(g_count, dtype=np.int64)
    rank[gorder] = np.arange(g_count)
    spin_rank = rank[gid]
    members = new[np.argsort(spin_rank, kind="stable")]
    member_off = np.concatenate([[0], np.cumsum(np.bincount(spin_rank, minlength=g_count))])

    header = np.array([n, g_count, len(spin_off) - 1, len(group_off) - 1,
                       len(all_ell) // n, len(ext_ell) // n], dtype=np.int32)
    parts = [header, h[perm], all_deg, all_ell, spin_off, ext_deg, ext_ell, group_off, member_off, members]
    blob = b"".join(np.ascontiguousarray(p, dtype=np.int32).tobytes() for p in parts)
    stats["spin_colors"] = len(spin_off) - 1
    stats["group_colors"] = len(group_off) - 1
    return blob, perm, gorder, stats
