"""F15 diamond-3d: full-chip native diamond glass with protocol-drawn bond signs.

Port of the suite's ``work/diamond.py``. A 52 x 52 x 16 diamond lattice is
placed on the chip at the integer shift that keeps the most live spins, then
the most couplers, then the smallest shift. The retained bonds, sorted by
spin pair, take a protocol draw from ``[-1000, 1000]``.
"""

from __future__ import annotations

from functools import lru_cache
from itertools import product

import numpy as np

from isingmark_structured import lattices, protocol, zephyr
from isingmark_structured.topology import SuiteSpec

DIMS = (52, 52, 16)


@lru_cache(maxsize=4)
def layout(suite: SuiteSpec) -> dict[str, np.ndarray]:
    """Select the largest clipped placement over all integer shifts in [-24, 24]."""
    native = suite.nodes
    physical_edges = native[suite.dense_edges]
    physical_count = zephyr.ZEPHYR_QUBITS
    available = np.zeros(physical_count + 1, dtype=bool)
    available[native] = True
    adjacent = np.zeros((physical_count + 1, physical_count + 1), dtype=bool)
    adjacent[physical_edges[:, 0], physical_edges[:, 1]] = True
    adjacent[physical_edges[:, 1], physical_edges[:, 0]] = True
    lattice_edges = lattices.diamond_edges(DIMS)
    all_sites = lattices.diamond_sites(DIMS)
    best: tuple[int, int, tuple[int, int], np.ndarray, np.ndarray] | None = None
    for across, down in product(range(-24, 25), repeat=2):
        shift = (across, down)
        labels = lattices.diamond_labels(DIMS, shift)
        upper = int(available[labels].sum())
        if best is None or upper >= best[0]:
            keep = adjacent[labels[lattice_edges[:, 0]], labels[lattice_edges[:, 1]]]
            alive = np.zeros(len(labels), dtype=bool)
            alive[lattice_edges[keep].ravel()] = True
            active, couplers = int(alive.sum()), int(keep.sum())
            if best is None or (-active, -couplers, shift) < (-best[0], -best[1], best[2]):
                best = (active, couplers, shift, labels, keep)
    if best is None:
        raise ValueError("no diamond placement")
    _, _, _, labels, keep = best
    mapped = labels[labels >= 0]
    if set(mapped.tolist()) != set(range(physical_count)) or len(mapped) != physical_count:
        raise ValueError("selected footprint does not cover the ideal chip exactly once")
    inverse = np.full(physical_count, -1, dtype=np.int64)
    inverse[native] = np.arange(len(native))
    raw_edges = inverse[labels[lattice_edges[keep]]]
    edges = np.asarray(
        sorted((min(u, v), max(u, v)) for u, v in raw_edges.tolist()), dtype=np.int64
    )
    if len({tuple(edge) for edge in edges.tolist()}) != len(edges) or np.any(edges < 0):
        raise ValueError("invalid mapped edge labels")
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
    )
    sites = np.full((len(native), 3), -1, dtype=np.int64)
    in_live = available[labels]
    sites[inverse[labels[in_live]]] = all_sites[in_live]
    return {"edges": edges, "sites": sites, "clusters": clusters}


def generate(suite: SuiteSpec, nonce: str) -> dict[str, np.ndarray]:
    """Draw signs on the fixed sorted kernel edges after one zero-field draw per spin."""
    selected = layout(suite)
    edges = selected["edges"].copy()
    return {
        "edges": edges,
        "j_milli": protocol.draw_couplings(nonce, suite.n_spins, len(edges), [-1000, 1000]),
        "h_milli": np.zeros(suite.n_spins, dtype=np.int64),
        "clusters": selected["clusters"].copy(),
        "sites": selected["sites"].copy(),
    }
