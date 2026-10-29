"""F2 strong-net: random strong-bond networks at three mean strong degrees.

Port of the suite's ``work/strongnet.py``. One uniform draw per native
coupler marks it strong when below ``density / 18.1``, so the three density
masks are nested. One protocol draw from the tri-match alphabet covers every
coupler; strong values stay and weak couplers keep their sign at 200 milli.
"""

from __future__ import annotations

import heapq

import numpy as np

from isingmark_structured import protocol
from isingmark_structured.families.tri_match import ALPHABET
from isingmark_structured.topology import SuiteSpec

DENSITIES = (1.5, 2, 3)
BLOCK_SIZE = 6


def build_blocks(edges: np.ndarray, couplings: np.ndarray, nodes: int) -> np.ndarray:
    """Six-spin exact blocks grown by largest boundary magnitude, covering every strong edge."""
    adjacency: list[list[tuple[int, int, int]]] = [[] for _ in range(nodes)]
    active: set[int] = set()
    strong = []
    for index, ((u, v), coupling) in enumerate(zip(edges, couplings, strict=True)):
        u, v, magnitude = int(u), int(v), abs(int(coupling))
        adjacency[u].append((-magnitude, index, v))
        adjacency[v].append((-magnitude, index, u))
        if magnitude >= 800:
            strong.append(index)
            active.update((u, v))
    groups: set[tuple[int, ...]] = set()
    covered: set[int] = set()

    def grow(seeds: tuple[int, ...]) -> None:
        members = set(seeds)
        frontier = [edge for node in seeds for edge in adjacency[node]]
        heapq.heapify(frontier)
        while len(members) < BLOCK_SIZE:
            if not frontier:
                raise ValueError("a physical component cannot fill a six-spin block")
            _, _, node = heapq.heappop(frontier)
            if node in members:
                continue
            members.add(node)
            for edge in adjacency[node]:
                if edge[2] not in members:
                    heapq.heappush(frontier, edge)
        group = tuple(sorted(members))
        groups.add(group)
        for node in group:
            for magnitude, index, neighbor in adjacency[node]:
                if magnitude <= -800 and neighbor in members:
                    covered.add(index)

    for node in sorted(active):
        grow((node,))
    for index in strong:
        if index not in covered:
            grow(tuple(int(node) for node in edges[index]))
    if len(covered) != len(strong):
        raise ValueError("a strong edge has no exact block")
    return np.asarray(sorted(groups), dtype=np.int64).reshape(-1, BLOCK_SIZE)


def generate(suite: SuiteSpec, nonce: str, density: float) -> dict[str, np.ndarray]:
    if density not in DENSITIES:
        raise ValueError("F2 densities are 1.5, 2, and 3")
    rng = protocol.nonce_rng(nonce)
    edges = suite.dense_edges.copy()
    strong = rng.random(len(edges)) < density / 18.1
    couplings = protocol.draw_couplings(nonce, suite.n_spins, len(edges), ALPHABET)
    couplings[~strong] = np.sign(couplings[~strong]) * 200
    return {
        "edges": edges,
        "j_milli": couplings,
        "h_milli": np.zeros(suite.n_spins, dtype=np.int64),
        "strong_mask": strong.astype(np.int64),
        "clusters": build_blocks(edges, couplings, suite.n_spins),
        "physical_nodes": suite.nodes.copy(),
    }
