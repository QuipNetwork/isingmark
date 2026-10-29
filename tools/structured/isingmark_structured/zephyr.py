"""Zephyr and Chimera coordinate helpers.

Exact ports of the parts of ``dwave.graphs`` (``zephyr_coordinates`` and the
single-tile Chimera to Zephyr sublattice mapping) and of
``quip_miner_dwave.embeddings`` that the suite generators call, so the
package needs no D-Wave dependency. The chip is ``Z(12, 4)``.
"""

from __future__ import annotations

from collections.abc import Callable

import numpy as np

ZEPHYR_M = 12
ZEPHYR_T = 4
# Qubits of the ideal Z(m, t): 4 t m (2 m + 1).
ZEPHYR_QUBITS = 4 * ZEPHYR_T * ZEPHYR_M * (2 * ZEPHYR_M + 1)

ChimeraNode = tuple[int, int, int, int]


def linear_to_zephyr(
    r: int, m: int = ZEPHYR_M, t: int = ZEPHYR_T
) -> tuple[int, int, int, int, int]:
    """``(u, w, k, j, z)`` of linear qubit ``r``."""
    r, z = divmod(r, m)
    r, j = divmod(r, 2)
    r, k = divmod(r, t)
    u, w = divmod(r, 2 * m + 1)
    return u, w, k, j, z


def zephyr_to_linear(
    u: int, w: int, k: int, j: int, z: int, m: int = ZEPHYR_M, t: int = ZEPHYR_T
) -> int:
    return (((u * (2 * m + 1) + w) * t + k) * 2 + j) * m + z


def zephyr_labels(
    u: np.ndarray,
    w: np.ndarray,
    k: np.ndarray,
    j: np.ndarray,
    z: np.ndarray,
    m: int = ZEPHYR_M,
    t: int = ZEPHYR_T,
) -> np.ndarray:
    """Vectorized ``embeddings.zephyr_label``: the linear label, or -1 outside the graph."""
    inside = (
        (w >= 0) & (w <= 2 * m) & (k >= 0) & (k < t) & (z >= 0) & (z < m) & ((j == 0) | (j == 1))
    )
    linear = (((u * (2 * m + 1) + w) * t + k) * 2 + j) * m + z
    return np.where(inside, linear, -1).astype(np.int64)


def chimera_edges(rows: int, columns: int, tile: int) -> list[tuple[ChimeraNode, ChimeraNode]]:
    """The couplers of ``chimera_graph(rows, columns, tile, coordinates=True)``.

    Nodes are ``(y, x, u, k)``. Edge order differs from ``dwave.graphs``; the
    suite only uses this list as a set.
    """
    edges: list[tuple[ChimeraNode, ChimeraNode]] = []
    for y in range(rows):
        for x in range(columns):
            for k in range(tile):
                for kk in range(tile):
                    edges.append(((y, x, 0, k), (y, x, 1, kk)))
                if y + 1 < rows:
                    edges.append(((y, x, 0, k), (y + 1, x, 0, k)))
                if x + 1 < columns:
                    edges.append(((y, x, 1, k), (y, x + 1, 1, k)))
    return edges


def chimera_nodes(rows: int, columns: int, tile: int) -> list[ChimeraNode]:
    return [
        (y, x, u, k)
        for y in range(rows)
        for x in range(columns)
        for u in range(2)
        for k in range(tile)
    ]


def chimera_to_zephyr_mapping(
    offset: tuple[int, int, int, int, int], m: int = ZEPHYR_M
) -> Callable[[ChimeraNode], int]:
    """``dwave.graphs`` single-tile sublattice mapping ``chimera(2m, 2m, t)`` into ``Z(m, t)``.

    ``offset`` is ``(t, y_offset, x_offset, k_offset0, k_offset1)``.
    """
    t, y_offset, x_offset, k_offset0, k_offset1 = offset

    def mapping(q: ChimeraNode) -> int:
        y, x, u, k = q
        if u:
            dw, kk = divmod(k + k_offset1, t)
            z, j = divmod(x + x_offset, 2)
            return zephyr_to_linear(u, y + y_offset + dw, kk, j, z, m, t)
        dw, kk = divmod(k + k_offset0, t)
        z, j = divmod(y + y_offset, 2)
        return zephyr_to_linear(u, x + x_offset + dw, kk, j, z, m, t)

    return mapping
