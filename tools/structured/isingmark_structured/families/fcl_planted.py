"""F4 fcl-planted: planted frustrated loops with whole-loop range rejection.

Port of the suite's ``work/fcl.py``. Random non-backtracking walks close
simple cycles of 3 to 12 spins; each cycle adds -1 to its bonds and +1 to one
random bond, and a cycle that would push any bond outside [-1, 1] is
rejected whole. ``floor(alpha * spins)`` loops are planted, then a random
planted state gauges the result. Only couplers with a nonzero sum appear.
"""

from __future__ import annotations

from collections import Counter

import numpy as np

from isingmark_structured import protocol
from isingmark_structured.topology import SuiteSpec

MAX_WALKS = 250000
DENSITIES_TENTHS = (2, 3)


def add_cycle(weights: np.ndarray, selected: list[int], af: int) -> bool:
    """Apply a whole cycle only when every new coefficient stays in range."""
    proposed = weights[selected] - 1
    proposed[af] += 2
    if np.any(np.abs(proposed) > 1):
        return False
    weights[selected] = proposed
    return True


def draw_loops(
    edges: np.ndarray, nodes: int, count: int, rng: np.random.Generator, max_walks: int = MAX_WALKS
) -> dict[str, np.ndarray]:
    neighbors: list[list[int]] = [[] for _ in range(nodes)]
    indices: dict[tuple[int, int], int] = {}
    for index, (a, b) in enumerate(edges):
        u, v = int(a), int(b)
        neighbors[u].append(v)
        neighbors[v].append(u)
        indices[(min(u, v), max(u, v))] = index
    for adjacent in neighbors:
        adjacent.sort()
    weights = np.zeros(len(edges), dtype=np.int64)
    members: list[int] = []
    offsets = [0]
    antiferromagnetic: list[int] = []
    rejected: Counter[str] = Counter()
    walks = 0
    while len(antiferromagnetic) < count and walks < max_walks:
        walks += 1
        path = [int(rng.integers(nodes))]
        seen = {path[0]: 0}
        while True:
            choices = [v for v in neighbors[path[-1]] if len(path) < 2 or v != path[-2]]
            if not choices:
                rejected["dead_end"] += 1
                break
            following = choices[int(rng.integers(len(choices)))]
            if following in seen:
                cycle = path[seen[following] :]
                if not 3 <= len(cycle) <= 12:
                    rejected["length"] += 1
                    break
                selected = [
                    indices[(min(u, v), max(u, v))]
                    for u, v in zip(cycle, cycle[1:] + cycle[:1], strict=True)
                ]
                af = int(rng.integers(len(cycle)))
                if not add_cycle(weights, selected, af):
                    rejected["range"] += 1
                    break
                members.extend(cycle)
                offsets.append(len(members))
                antiferromagnetic.append(af)
                break
            seen[following] = len(path)
            path.append(following)
    if len(antiferromagnetic) != count:
        raise RuntimeError(
            f"loop target {count} unmet after {walks} walks: {len(antiferromagnetic)} accepted"
        )
    return {
        "weights": weights,
        "nodes": np.asarray(members, dtype=np.int64),
        "offsets": np.asarray(offsets, dtype=np.int64),
        "af_positions": np.asarray(antiferromagnetic, dtype=np.int64),
        "walks": np.asarray([walks], dtype=np.int64),
        "rejected_length": np.asarray([rejected["length"]], dtype=np.int64),
        "rejected_range": np.asarray([rejected["range"]], dtype=np.int64),
        "rejected_dead_end": np.asarray([rejected["dead_end"]], dtype=np.int64),
    }


def generate(suite: SuiteSpec, nonce: str, density_tenths: int) -> dict[str, np.ndarray]:
    if density_tenths not in DENSITIES_TENTHS:
        raise ValueError("the F4 densities are two and three tenths")
    rng = protocol.nonce_rng(nonce)
    edges = suite.dense_edges
    planted = rng.choice(np.asarray([-1, 1], dtype=np.int8), size=suite.n_spins)
    loops = draw_loops(edges, suite.n_spins, suite.n_spins * density_tenths // 10, rng)
    keep = loops["weights"] != 0
    model = {
        "edges": edges[keep],
        "j_milli": 1000 * loops["weights"][keep] * np.prod(planted[edges[keep]], axis=1),
        "h_milli": np.zeros(suite.n_spins, dtype=np.int64),
        "physical_nodes": suite.nodes.copy(),
        "plant_spins": planted,
        "raw_edge_weights": loops["weights"],
    }
    for key in (
        "nodes",
        "offsets",
        "af_positions",
        "walks",
        "rejected_length",
        "rejected_range",
        "rejected_dead_end",
    ):
        model[f"loop_{key}"] = loops[key]
    return model
