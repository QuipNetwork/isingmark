"""F16 cubic-no-dimer: native cubic layout with independent physical spin bonds.

Port of the suite's ``work/cubic.py``. The 26 x 26 x 8 cubic dimer layout is
placed on the chip at the integer shift that keeps the most live spins, then
the most couplers, then the smallest shift. Every retained physical coupler,
the former dimer bonds included, takes its own protocol draw from
``[-1000, 1000]`` in sorted spin-pair order.
"""

from __future__ import annotations

from functools import lru_cache
from itertools import product

import numpy as np

from isingmark_structured import lattices, protocol, zephyr
from isingmark_structured.topology import SuiteSpec

DIMS = (26, 26, 8)


def lattice_edges(dims: lattices.Dims) -> tuple[np.ndarray, np.ndarray]:
    """Retain the first emitted edge of each logical bond and all intra-site edges."""
    bonds, axes = lattices.cubic_logical_bonds(dims)
    edges, _, owners = lattices.cubic_dimer_edges(dims, np.full(len(bonds), 1000, dtype=np.int64))
    keep = np.ones(len(edges), dtype=bool)
    keep[1:] = (owners[1:] < 0) | (owners[1:] != owners[:-1])
    edge_axis = np.full(len(edges), -1, dtype=np.int64)
    edge_axis[owners >= 0] = axes[owners[owners >= 0]]
    if np.any(edge_axis[~keep] != 2):
        raise ValueError("a removed duplicate is not a z coupler")
    return edges[keep], edge_axis[keep]


@lru_cache(maxsize=4)
def layout(suite: SuiteSpec) -> dict[str, np.ndarray]:
    """Search the clipped footprint over all integer shifts in [-24, 24]."""
    native = suite.nodes
    size = zephyr.ZEPHYR_QUBITS
    available = np.zeros(size + 1, dtype=bool)
    available[native] = True
    adjacency = np.zeros((size + 1, size + 1), dtype=bool)
    physical = native[suite.dense_edges]
    adjacency[physical[:, 0], physical[:, 1]] = True
    adjacency[physical[:, 1], physical[:, 0]] = True
    ideal_edges, axes = lattice_edges(DIMS)
    best: tuple[int, int, tuple[int, int], np.ndarray, np.ndarray] | None = None
    for across, down in product(range(-24, 25), repeat=2):
        shift = (across, down)
        labels = lattices.cubic_dimer_labels(DIMS, shift)
        upper = int(available[labels].sum())
        if best is None or upper >= best[0]:
            keep = adjacency[labels[ideal_edges[:, 0]], labels[ideal_edges[:, 1]]]
            alive = np.zeros(len(labels), dtype=bool)
            alive[ideal_edges[keep].ravel()] = True
            active, couplers = int(alive.sum()), int(keep.sum())
            if best is None or (-active, -couplers, shift) < (-best[0], -best[1], best[2]):
                best = (active, couplers, shift, labels, keep)
    if best is None:
        raise ValueError("no cubic placement")
    _, _, _, labels, keep = best
    mapped = labels[labels >= 0]
    if len(mapped) != len(np.unique(mapped)) or len(mapped) > size // 2:
        raise ValueError("the cubic footprint is not injective within half the chip")
    inverse = np.full(size, -1, dtype=np.int64)
    inverse[native] = np.arange(len(native))
    raw = np.sort(inverse[labels[ideal_edges[keep]]], axis=1)
    order = np.lexsort((raw[:, 1], raw[:, 0]))
    edges = raw[order]
    edge_axis = axes[keep][order]
    if np.any(edges < 0) or len(np.unique(edges, axis=0)) != len(edges):
        raise ValueError("invalid or repeated physical edge")
    count = int(np.prod(DIMS))
    index = np.arange(2 * count, dtype=np.int64)
    logical = index % count
    all_sites = np.column_stack(
        (
            logical % DIMS[0],
            logical // DIMS[0] % DIMS[1],
            logical // (DIMS[0] * DIMS[1]),
            index // count,
        )
    )
    sites = np.full((suite.n_spins, 4), -1, dtype=np.int64)
    live = available[labels]
    sites[inverse[labels[live]]] = all_sites[live]
    neighbors: dict[int, set[int]] = {}
    for u, v in edges.tolist():
        neighbors.setdefault(u, set()).add(v)
        neighbors.setdefault(v, set()).add(u)
    clusters = np.asarray(
        [
            [node, *sorted(neighbors[node])]
            for node in sorted(neighbors)
            if len(neighbors[node]) == 4
        ],
        dtype=np.int64,
    ).reshape(-1, 5)
    return {
        "edges": edges,
        "edge_axis": edge_axis,
        "sites": sites,
        "clusters": clusters,
        "pairs": edges[edge_axis == -1].copy(),
    }


def generate(suite: SuiteSpec, nonce: str) -> dict[str, np.ndarray]:
    """Draw all retained edge signs independently after one zero-field draw per spin."""
    selected = layout(suite)
    edges = selected["edges"].copy()
    return {
        "edges": edges,
        "j_milli": protocol.draw_couplings(nonce, suite.n_spins, len(edges), [-1000, 1000]),
        "h_milli": np.zeros(suite.n_spins, dtype=np.int64),
        "clusters": selected["clusters"].copy(),
        "pairs": selected["pairs"].copy(),
        "sites": selected["sites"].copy(),
        "edge_axis": selected["edge_axis"].copy(),
        "physical_nodes": suite.nodes.copy(),
    }
