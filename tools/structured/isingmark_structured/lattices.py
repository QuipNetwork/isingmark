"""Diamond and cubic lattices placed on the chip.

Exact ports of ``quip_miner_dwave.lattices`` (site numbering and bond order
of King et al.'s ``quenchlib``) and of the label formulas of
``quip_miner_dwave.embeddings``, vectorized over sites.
"""

from __future__ import annotations

import itertools

import numpy as np

from isingmark_structured import zephyr

Dims = tuple[int, int, int]
Shift = tuple[int, int]

# Column (x, y) of the diamond lattice holds one site in each block of four
# levels, at level 4 zc + _COLUMN_LEVEL[x mod 2][y mod 2].
_COLUMN_LEVEL = [[1, 0], [2, 3]]
CUBIC_DIMER_MILLI = -2000


def diamond_sites(dims: Dims) -> np.ndarray:
    """``(x, y, level)`` of each site, shape ``(n, 3)``, in site-number order."""
    lx, ly, lz = dims
    sites = [
        (x, y, 4 * zc + _COLUMN_LEVEL[x % 2][y % 2])
        for y, zc, x in itertools.product(range(ly), range(lz // 4), range(lx))
    ]
    return np.asarray(sites, dtype=np.int64).reshape(-1, 3)


def diamond_edges(dims: Dims) -> np.ndarray:
    """Bonds of the diamond lattice, shape ``(m, 2)``, sorted by site pair."""
    lz = dims[2]
    if lz % 4 != 0 or lz < 4:
        raise ValueError(f"Lz must be a multiple of 4; got {lz}")
    numbers = {(x, y, level): i for i, (x, y, level) in enumerate(diamond_sites(dims).tolist())}
    edges: list[tuple[int, int]] = []
    for (x, y, z), u in numbers.items():
        d = z & 1
        for dx, dy in ((d, 1 - d), (-d, d - 1)):
            v = numbers.get((x + dx, y + dy, (z + 1) % lz))
            if v is not None:
                edges.append((min(u, v), max(u, v)))
    return np.asarray(sorted(edges), dtype=np.int64).reshape(-1, 2)


def diamond_labels(dims: Dims, shift: Shift) -> np.ndarray:
    """The qubit label of each diamond site on an ideal ``Z(12, 4)``, or -1."""
    ly = dims[1]
    sites = diamond_sites(dims)
    x, y, level = sites[:, 0], sites[:, 1], sites[:, 2]
    if np.any(level // 4 >= zephyr.ZEPHYR_T):
        raise ValueError(
            f"a depth of {dims[2]} needs more than {zephyr.ZEPHYR_T} wires for each line"
        )
    k = level // 4
    even = level % 2 == 0
    v_column = (x + y - 1) // 2 + shift[0]
    v_row = (x - y - 1) // 2 + ly // 2 + shift[1]
    vertical = zephyr.zephyr_labels(np.zeros_like(x), v_column, k, v_row % 2, v_row // 2)
    h_row = (x - y) // 2 + ly // 2 + shift[1]
    h_column = (x + y) // 2 - 1 + shift[0]
    horizontal = zephyr.zephyr_labels(np.ones_like(x), h_row, k, h_column % 2, h_column // 2)
    return np.where(even, vertical, horizontal)


def cubic_logical_bonds(dims: Dims) -> tuple[np.ndarray, np.ndarray]:
    """Bonds of the simple cubic lattice in draw order (x, then y, then z) and their axes."""
    lx, ly, lz = dims
    if min(lx, ly) < 2:
        raise ValueError(f"Lx and Ly must be at least 2; got {dims}")

    def site(x: int, y: int, z: int) -> int:
        return x + y * lx + z * lx * ly

    bonds: list[tuple[int, int]] = []
    axis: list[int] = []
    cells = list(itertools.product(range(lz), range(ly), range(lx)))
    for z, y, x in cells:
        if x < lx - 1:
            bonds.append((site(x, y, z), site(x + 1, y, z)))
            axis.append(0)
    for z, y, x in cells:
        if y < ly - 1:
            bonds.append((site(x, y, z), site(x, y + 1, z)))
            axis.append(1)
    for z, y, x in cells:
        if z < lz - 1:
            bonds.append((site(x, y, z), site(x, y, z + 1)))
            axis.append(2)
        elif lz > 2:
            bonds.append((site(x, y, z), site(x, y, 0)))
            axis.append(2)
    return np.asarray(bonds, dtype=np.int64).reshape(-1, 2), np.asarray(axis, dtype=np.int64)


def cubic_dimer_edges(
    dims: Dims, bond_milli: np.ndarray
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """True edges and couplings of the cubic dimer lattice, dimers first, and the logical bond
    that made each edge (-1 for a dimer)."""
    lx, ly, lz = dims
    n = lx * ly * lz
    bonds, axis = cubic_logical_bonds(dims)
    drawn = np.asarray(bond_milli, dtype=np.int64)
    if drawn.shape != (len(bonds),):
        raise ValueError(f"need one coupling for each of {len(bonds)} bonds; got {drawn.shape}")
    if np.any(drawn % 2):
        raise ValueError("drawn couplings must be even so that a z half is a whole number")
    edges: list[tuple[int, int]] = [(u, u + n) for u in range(n)]
    couplings: list[int] = [CUBIC_DIMER_MILLI] * n
    made_by: list[int] = [-1] * n
    for index, ((u, v), ax, j) in enumerate(
        zip(bonds.tolist(), axis.tolist(), drawn.tolist(), strict=True)
    ):
        if ax == 0:
            edges.append((u, v))
            couplings.append(j)
            made_by.append(index)
        elif ax == 1:
            edges.append((u + n, v + n))
            couplings.append(j)
            made_by.append(index)
        else:
            edges += [(u, v + n), (u + n, v)]
            couplings += [j // 2, j // 2]
            made_by += [index, index]
    return (
        np.asarray(edges, dtype=np.int64).reshape(-1, 2),
        np.asarray(couplings, dtype=np.int64),
        np.asarray(made_by, dtype=np.int64),
    )


def cubic_dimer_labels(dims: Dims, shift: Shift) -> np.ndarray:
    """The qubit label of each cubic dimer spin on an ideal ``Z(12, 4)``, or -1."""
    lx, ly, lz = dims
    if lz > 2 * zephyr.ZEPHYR_T:
        raise ValueError(f"a depth of {lz} needs more than {zephyr.ZEPHYR_T} wires for each line")
    n = lx * ly * lz
    site = np.arange(n, dtype=np.int64)
    x, y, z = site % lx, site // lx % ly, site // (lx * ly)
    row = 2 * x + shift[1]
    column = 2 * y + shift[0]
    labels = np.full(2 * n, -1, dtype=np.int64)
    labels[:n] = zephyr.zephyr_labels(np.zeros_like(x), column + z % 2, z // 2, row % 2, row // 2)
    labels[n:] = zephyr.zephyr_labels(np.ones_like(x), row + z % 2, z // 2, column % 2, column // 2)
    return labels
