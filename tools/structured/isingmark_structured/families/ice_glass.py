"""F11 ice-glass: nonce-defined corner-sharing tetrahedra on the live Zephyr topology.

Port of the suite's ``work/ice.py``. A random maximum bipartite matching pairs
vertical and horizontal odd couplers into tetrahedra whose six bonds are
+1000 milli. Every other coupler takes a protocol draw from
``[-background, background]``, and a random physical gauge hides the ice rule.
"""

from __future__ import annotations

from collections import Counter
from functools import lru_cache
from itertools import combinations

import numpy as np
from scipy.sparse import csr_matrix
from scipy.sparse.csgraph import maximum_bipartite_matching

from isingmark_structured import protocol, zephyr
from isingmark_structured.topology import SuiteSpec

BACKGROUNDS = (125, 200)


@lru_cache(maxsize=4)
def odd_pairs(suite: SuiteSpec) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """``(vertical, horizontal, candidates)``: odd couplers by orientation, in protocol edge
    order, and the (vertical, horizontal) index pairs that close a tetrahedron."""
    labels = [zephyr.linear_to_zephyr(int(q)) for q in suite.nodes]
    neighbors: list[set[int]] = [set() for _ in range(suite.n_spins)]
    for u, v in suite.dense_edges:
        neighbors[u].add(int(v))
        neighbors[v].add(int(u))
    odd = [
        (int(u), int(v))
        for u, v in suite.dense_edges
        if labels[u][0] == labels[v][0] and labels[u][3] != labels[v][3]
    ]
    vertical = np.asarray([e for e in odd if labels[e[0]][0] == 0], dtype=np.int64)
    horizontal = np.asarray([e for e in odd if labels[e[0]][0] == 1], dtype=np.int64)
    horizontal_index = {(int(min(e)), int(max(e))): i for i, e in enumerate(horizontal)}
    pairs = []
    for i, (u, v) in enumerate(vertical):
        for edge in combinations(sorted(neighbors[u] & neighbors[v]), 2):
            if edge in horizontal_index:
                pairs.append((i, horizontal_index[edge]))
    return vertical, horizontal, np.asarray(pairs, dtype=np.int64)


def generate(suite: SuiteSpec, nonce: str, background_milli: int) -> dict[str, np.ndarray]:
    if background_milli not in BACKGROUNDS:
        raise ValueError("F11 backgrounds are 125 or 200 milli")
    rng = protocol.nonce_rng(nonce)
    vertical, horizontal, candidates = odd_pairs(suite)
    edges = suite.dense_edges
    vorder = rng.permutation(len(vertical))
    horder = rng.permutation(len(horizontal))
    vinverse, hinverse = np.argsort(vorder), np.argsort(horder)
    graph = csr_matrix(
        (
            np.ones(len(candidates), dtype=np.int8),
            (vinverse[candidates[:, 0]], hinverse[candidates[:, 1]]),
        ),
        shape=(len(vertical), len(horizontal)),
    )
    matching = maximum_bipartite_matching(graph, perm_type="column")
    tetrahedra = np.asarray(
        [[*vertical[vorder[i]], *horizontal[horder[j]]] for i, j in enumerate(matching) if j >= 0],
        dtype=np.int64,
    )
    counts = Counter(
        tuple(sorted(edge)) for tetrahedron in tetrahedra for edge in combinations(tetrahedron, 2)
    )
    if max(counts.values()) != 1:
        raise ValueError("tetrahedra share an edge")
    if np.bincount(tetrahedra.ravel(), minlength=suite.n_spins).max() > 2:
        raise ValueError("a spin belongs to more than two tetrahedra")
    strong = np.asarray([tuple(sorted(edge)) in counts for edge in edges])
    if int(strong.sum()) != 6 * len(tetrahedra):
        raise ValueError("a tetrahedron uses a coupler outside the working graph")
    couplings = np.full(len(edges), 1000, dtype=np.int64)
    couplings[~strong] = protocol.draw_couplings(
        nonce, suite.n_spins, int((~strong).sum()), [-background_milli, background_milli]
    )
    gauge = rng.choice(np.asarray([-1, 1], dtype=np.int8), size=suite.n_spins)
    couplings *= np.prod(gauge[edges], axis=1)
    return {
        "edges": edges.copy(),
        "j_milli": couplings,
        "h_milli": np.zeros(suite.n_spins, dtype=np.int64),
        "tetrahedra": tetrahedra,
        "gauge": gauge,
        "physical_nodes": suite.nodes.copy(),
    }
