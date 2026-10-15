"""F1 tri-match: three disjoint matchings with a frustrated strong backbone.

Port of the suite's ``work/trimatch.py``. Three greedy maximal matchings on
fresh random edge orders own the strong couplers. One protocol draw from
``[-1000, -800] + [800, 1000]`` covers every native coupler; strong values stay
and every other coupler keeps its sign at the background magnitude (250 milli
for ratio 4, 167 milli for ratio 6).
"""

from __future__ import annotations

from functools import lru_cache

import numpy as np

from isingmark_structured import protocol, zephyr
from isingmark_structured.topology import SuiteSpec

ALPHABET = [*range(-1000, -799), *range(800, 1001)]
BACKGROUNDS = {4: 250, 6: 167}


@lru_cache(maxsize=4)
def orientation(suite: SuiteSpec) -> np.ndarray:
    """The Zephyr orientation ``u`` (0 vertical, 1 horizontal) of each spin."""
    return np.asarray([zephyr.linear_to_zephyr(int(q))[0] for q in suite.nodes], dtype=np.int64)


def generate(suite: SuiteSpec, nonce: str, ratio: int) -> dict[str, np.ndarray]:
    if ratio not in BACKGROUNDS:
        raise ValueError("F1 ratios are four and six")
    rng = protocol.nonce_rng(nonce)
    sides = orientation(suite)
    edges = suite.dense_edges.copy()
    owner = np.full(len(edges), -1, dtype=np.int64)
    for index in range(3):
        occupied = np.zeros(suite.n_spins, dtype=bool)
        for edge_index in rng.permutation(len(edges)):
            u, v = edges[edge_index]
            if owner[edge_index] < 0 and not occupied[u] and not occupied[v]:
                owner[edge_index] = index
                occupied[u] = occupied[v] = True
        selected = edges[owner == index]
        mixed = sides[selected[:, 0]] != sides[selected[:, 1]]
        if not mixed.any() or mixed.all():
            raise ValueError(f"matching {index} lacks one required coupler type")
    couplings = protocol.draw_couplings(nonce, suite.n_spins, len(edges), ALPHABET)
    couplings[owner < 0] = np.sign(couplings[owner < 0]) * BACKGROUNDS[ratio]
    neighbors: list[set[int]] = [set() for _ in range(suite.n_spins)]
    for u, v in edges[owner >= 0]:
        neighbors[u].add(int(v))
        neighbors[v].add(int(u))
    clusters = np.asarray(
        [[node, *sorted(links)] for node, links in enumerate(neighbors) if len(links) == 3],
        dtype=np.int64,
    ).reshape(-1, 4)
    return {
        "edges": edges,
        "j_milli": couplings,
        "h_milli": np.zeros(suite.n_spins, dtype=np.int64),
        "matching_index": owner,
        "clusters": clusters,
        "physical_nodes": suite.nodes.copy(),
    }
