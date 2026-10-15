"""F5 dcl-k44: nonce-defined deceptive cluster loops on intact native Chimera cells.

Port of the suite's ``work/dcl.py``. Intact K4,4 cells of a Chimera
sublattice of the chip are rigid ferromagnetic clusters at ``-cell_milli``.
Random walks on the cell graph plant frustrated loops whose bonds become
bundles of four +-1000 milli couplers, and a random physical gauge hides the
construction. Lambda 5 is 200 milli and lambda 7 is 143 milli.
"""

from __future__ import annotations

from collections import defaultdict
from functools import lru_cache
from itertools import product
from typing import Any

import numpy as np

from isingmark_structured import protocol, zephyr
from isingmark_structured.topology import SuiteSpec

MAX_LAYOUT_ATTEMPTS = 1000
MAX_WALKS_PER_ATTEMPT = 10000
CELL_MILLI = (200, 143)


class LoopBudgetExceeded(ValueError):
    """A complete loop set was not found within the construction budget."""


@lru_cache(maxsize=64)
def layout(suite: SuiteSpec, offset: tuple[int, int, int, int, int]) -> dict[str, np.ndarray]:
    """Map intact K4,4 cells at a native sublattice offset."""
    physical = {int(node): i for i, node in enumerate(suite.nodes)}
    allowed = {(min(int(u), int(v)), max(int(u), int(v))) for u, v in suite.dense_edges}
    mapping = zephyr.chimera_to_zephyr_mapping(offset)
    mapped = {node: physical.get(mapping(node)) for node in zephyr.chimera_nodes(24, 24, 4)}
    cells, nodes = [], []
    for row in range(24):
        for column in range(24):
            values = [mapped[row, column, side, k] for side in range(2) for k in range(4)]
            selected = [value for value in values if value is not None]
            if len(selected) != len(values):
                continue
            if all((min(u, v), max(u, v)) in allowed for u in selected[:4] for v in selected[4:]):
                cells.append((row, column))
                nodes.append(selected)
    cell_index = {cell: i for i, cell in enumerate(cells)}
    bundles: defaultdict[tuple[int, int], list[tuple[int, int]]] = defaultdict(list)
    for u, v in zephyr.chimera_edges(24, 24, 4):
        left, right = cell_index.get(u[:2]), cell_index.get(v[:2])
        if left is None or right is None or left == right:
            continue
        a, b = mapped[u], mapped[v]
        if a is None or b is None:
            raise ValueError("retained cell has a missing qubit")
        bundles[(min(left, right), max(left, right))].append((min(a, b), max(a, b)))
    eligible = sorted(
        edge
        for edge, bundle in bundles.items()
        if len(bundle) == 4 and all(pair in allowed for pair in bundle)
    )
    return {
        "coordinates": np.asarray(cells, dtype=np.int64),
        "nodes": np.asarray(nodes, dtype=np.int64),
        "cell_edges": np.asarray(eligible, dtype=np.int64),
        "bundles": np.asarray([sorted(bundles[edge]) for edge in eligible], dtype=np.int64),
    }


@lru_cache(maxsize=4)
def ranked_layouts(suite: SuiteSpec) -> tuple[tuple[tuple[int, ...], dict[str, np.ndarray]], ...]:
    """Rank native placements by retained cells, eligible links, then offset."""
    placements = [
        ((4, 0, 0, a, b), layout(suite, (4, 0, 0, a, b))) for a, b in product(range(5), repeat=2)
    ]
    return tuple(
        sorted(
            placements,
            key=lambda item: (-len(item[1]["nodes"]), -len(item[1]["cell_edges"]), item[0]),
        )
    )


def draw_loops(
    edges: np.ndarray, nodes: int, count: int, rng: np.random.Generator, max_attempts: int = 100000
) -> dict[str, np.ndarray]:
    """Accumulate frustrated simple cycles while keeping every edge in [-1, 1]."""
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
    offsets = [0]
    members: list[int] = []
    antiferromagnetic: list[int] = []
    walks = 0
    while len(antiferromagnetic) < count and walks < max_attempts:
        walks += 1
        path = [int(rng.integers(nodes))]
        seen = {path[0]: 0}
        while True:
            choices = [v for v in neighbors[path[-1]] if len(path) < 2 or v != path[-2]]
            if not choices:
                break
            following = choices[int(rng.integers(len(choices)))]
            if following in seen:
                cycle = path[seen[following] :]
                if len(cycle) < 4:
                    break
                selected = [
                    indices[(min(u, v), max(u, v))]
                    for u, v in zip(cycle, cycle[1:] + cycle[:1], strict=True)
                ]
                af_position = int(rng.integers(len(cycle)))
                proposed = weights[selected] - 1
                proposed[af_position] += 2
                if np.any(np.abs(proposed) > 1):
                    break
                weights[selected] = proposed
                members.extend(cycle)
                offsets.append(len(members))
                antiferromagnetic.append(af_position)
                break
            seen[following] = len(path)
            path.append(following)
    if len(antiferromagnetic) != count:
        raise LoopBudgetExceeded(
            f"loop target {count} unmet after {walks} walks: {len(antiferromagnetic)} accepted"
        )
    return {
        "weights": weights,
        "nodes": np.asarray(members, dtype=np.int64),
        "offsets": np.asarray(offsets, dtype=np.int64),
        "af_positions": np.asarray(antiferromagnetic, dtype=np.int64),
        "walks": np.asarray([walks], dtype=np.int64),
    }


def generate(suite: SuiteSpec, nonce: str, cell_milli: int) -> dict[str, np.ndarray]:
    """Draw the common loops and gauge, then assign the requested cell scale."""
    if cell_milli not in CELL_MILLI:
        raise ValueError("DCL cell magnitude must be 200 or 143 milli")
    rng = protocol.nonce_rng(nonce)
    placements = ranked_layouts(suite)
    chosen: tuple[int, tuple[int, ...], dict[str, np.ndarray], dict[str, np.ndarray]] | None = None
    for attempt in range(MAX_LAYOUT_ATTEMPTS):
        offset, native = placements[attempt % len(placements)]
        try:
            loops = draw_loops(
                native["cell_edges"],
                len(native["nodes"]),
                24 * len(native["nodes"]) // 100,
                rng,
                max_attempts=MAX_WALKS_PER_ATTEMPT,
            )
        except LoopBudgetExceeded:
            continue
        chosen = (attempt, offset, native, loops)
        break
    if chosen is None:
        raise LoopBudgetExceeded(
            f"no complete native loop set after {MAX_LAYOUT_ATTEMPTS} attempts: {nonce}"
        )
    attempt, offset, native, loops = chosen
    cells = native["nodes"]
    weights: dict[tuple[int, int], int] = {
        (min(int(u), int(v)), max(int(u), int(v))): -cell_milli
        for nodes in cells
        for u in nodes[:4]
        for v in nodes[4:]
    }
    for value, bundle in zip(loops["weights"], native["bundles"], strict=True):
        if value:
            for u, v in bundle:
                weights[(int(u), int(v))] = 1000 * int(value)
    edges = np.asarray(sorted(weights), dtype=np.int64)
    coupling = np.asarray([weights[(int(u), int(v))] for u, v in edges], dtype=np.int64)
    gauge = rng.choice(np.asarray([-1, 1], dtype=np.int8), size=suite.n_spins)
    active = loops["weights"] != 0
    model: dict[str, Any] = {
        "edges": edges,
        "j_milli": coupling * np.prod(gauge[edges], axis=1),
        "h_milli": np.zeros(suite.n_spins, dtype=np.int64),
        "clusters": cells.copy(),
        "layout_offset": np.asarray(offset, dtype=np.int64),
        "cell_coordinates": native["coordinates"].copy(),
        "cell_edges": native["cell_edges"][active],
        "cell_j": loops["weights"][active],
        "loop_nodes": loops["nodes"],
        "loop_offsets": loops["offsets"],
        "loop_af_positions": loops["af_positions"],
        "loop_walks": loops["walks"],
        "loop_attempt": np.asarray([attempt + 1], dtype=np.int64),
        "loop_total_walks": loops["walks"] + attempt * MAX_WALKS_PER_ATTEMPT,
        "gauge": gauge,
        "physical_nodes": suite.nodes.copy(),
    }
    return model
