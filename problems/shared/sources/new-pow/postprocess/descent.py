"""Zero-temperature descent and seeded low-temperature anneals on archived QPU reads.

Models and targets come from ../rescore.py (CELLS, energy). Archives are located the same way as
../notion/build_pages.py (base_archives). Energies are integer milli. Every final state is
rescored with rescore.energy before it is reported.
"""
from __future__ import annotations

import ctypes
import itertools
import json
import math
import sys
import time
from pathlib import Path

import dimod
import numpy as np
import scipy.sparse as sp
from dwave.samplers import SimulatedAnnealingSampler, SteepestDescentSolver

NP_DIR = Path("/home/carback1/quip-data/regimes/new-pow")
sys.path.insert(0, str(NP_DIR))
sys.path.insert(0, str(NP_DIR / "gpu" / "scripts"))
import prep_groups  # noqa: E402
import rescore  # noqa: E402

REGIMES = NP_DIR.parent
ARCHIVE_ROOTS = [NP_DIR / "qpu-recap" / "qpu", REGIMES / "qpu-f17" / "qpu", REGIMES / "qpu-suite" / "qpu"]
BASE_ARCHIVES = {
    "cbfm-p-f17": [REGIMES / "qpu-f17" / "qpu" / "cbfm-p"],
    "cbfm-p": [REGIMES / "qpu-suite" / "qpu" / "cbfm-p"],
    "parity-k44": [NP_DIR / "qpu-recap" / "qpu" / "parity-k44"],
}


def page_cells() -> list[str]:
    pages = {p.stem for p in (NP_DIR / "notion" / "pages").glob("*.md")}
    return [c for c in rescore.CELLS if c in pages]


def targets(cell: str) -> list[dict]:
    path, keys = rescore.CELLS[cell]
    data = json.loads(path.read_text())
    return [row for key in keys for row in data[key]]


def archive(cell: str, nonce: str) -> Path | None:
    bases = BASE_ARCHIVES.get(cell) or [r / cell for r in ARCHIVE_ROOTS if (r / cell).is_dir()]
    for base in bases:
        f = base / "qpu-80" / f"{nonce}.npz"
        if f.exists():
            return f
    return None


def load_model(path: str) -> dict[str, np.ndarray]:
    with np.load(path, allow_pickle=False) as d:
        return {k: d[k] for k in d.files}


class Ising:
    """Integer Ising model with a symmetric CSR coupling matrix for vectorized local fields."""

    def __init__(self, model: dict[str, np.ndarray]):
        self.model = model
        self.h = model["h_milli"].astype(np.int64)
        self.n = len(self.h)
        e = model["edges"].astype(np.int64)
        j = model["j_milli"].astype(np.int64)
        self.edges, self.j = e, j
        rows = np.concatenate([e[:, 0], e[:, 1]])
        cols = np.concatenate([e[:, 1], e[:, 0]])
        self.jmat = sp.csr_matrix((np.concatenate([j, j]), (rows, cols)), shape=(self.n, self.n), dtype=np.int64)
        keys = np.minimum(e[:, 0], e[:, 1]) * self.n + np.maximum(e[:, 0], e[:, 1])
        self._order = np.argsort(keys)
        self._sorted = keys[self._order]

    def energies(self, s: np.ndarray) -> np.ndarray:
        s = s.astype(np.int64)
        return (s[:, self.edges[:, 0]] * s[:, self.edges[:, 1]]) @ self.j + s @ self.h

    def flip_deltas(self, s: np.ndarray) -> np.ndarray:
        """Energy change of each single flip, shape (reads, n)."""
        s = s.astype(np.int64)
        fields = (self.jmat @ s.T).T + self.h
        return -2 * s * fields

    def edge_ids(self, u: np.ndarray, v: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
        k = np.minimum(u, v) * self.n + np.maximum(u, v)
        pos = np.minimum(np.searchsorted(self._sorted, k), len(self._sorted) - 1)
        return self._order[pos], self._sorted[pos] == k

    def bqm(self) -> dimod.BinaryQuadraticModel:
        return dimod.BinaryQuadraticModel.from_numpy_vectors(
            self.h.astype(float), (self.edges[:, 0], self.edges[:, 1], self.j.astype(float)), 0.0, dimod.SPIN)

    def weakest(self) -> int:
        vals = np.concatenate([np.abs(self.j), np.abs(self.h)])
        return int(vals[vals > 0].min())


# ---------------------------------------------------------------- move sets


def natural_moves(ising: Ising) -> tuple[list[tuple[int, ...]], dict[str, int]]:
    """Multi-spin moves that the cell's structure suggests, beyond single flips.

    Every coupled pair (covers vertical pairs on stacked ice, dimers and the 2-spin moves inside a
    tile), the 3- and 4-spin subsets of every tetrahedron (tile-cluster moves), every listed
    cluster, strong block and pair, and every rigid group from gpu/scripts/prep_groups.py.
    """
    m = ising.model
    moves: set[tuple[int, ...]] = set()
    counts: dict[str, int] = {}

    def add(kind: str, tuples) -> None:
        before = len(moves)
        for t in tuples:
            t = tuple(sorted({int(x) for x in t if x >= 0}))
            if len(t) >= 2:
                moves.add(t)
        counts[kind] = len(moves) - before

    add("edge_pairs", ising.edges.tolist())
    if "tetrahedra" in m and len(m["tetrahedra"]):
        tet = m["tetrahedra"].astype(np.int64)
        subsets = [c for k in (3, 4) for c in itertools.combinations(range(4), k)]
        add("tile_subsets", (row[list(c)] for row in tet for c in subsets))
    for key in ("clusters", "strong_blocks", "pairs", "vertical_bonds"):
        if key in m and m[key].ndim == 2 and m[key].dtype.kind in "iu" and len(m[key]):
            add(key, m[key].astype(np.int64).tolist())
    gid = prep_groups.extract_groups(m)
    sizes = np.bincount(gid)
    big = np.flatnonzero(sizes > 1)
    if len(big):
        order = np.argsort(gid, kind="stable")
        bounds = np.concatenate([[0], np.cumsum(sizes)])
        add("rigid_groups", (order[bounds[g]:bounds[g + 1]].tolist() for g in big))
    return sorted(moves), counts


class MoveSet:
    """Sparse operators that give the energy change of every move for every read.

    delta(move) = sum of single-flip deltas of its members + 4 * sum over internal edges J s_a s_b.
    """

    def __init__(self, ising: Ising, moves: list[tuple[int, ...]], with_singles: bool = True):
        n = ising.n
        allm = ([(i,) for i in range(n)] if with_singles else []) + list(moves)
        self.members = allm
        cols, rows = [], []
        ia, ib, ij, im = [], [], [], []
        for k, t in enumerate(allm):
            rows.extend(t)
            cols.extend([k] * len(t))
            if len(t) > 1:
                a = np.array([p[0] for p in itertools.combinations(t, 2)], dtype=np.int64)
                b = np.array([p[1] for p in itertools.combinations(t, 2)], dtype=np.int64)
                eid, ok = ising.edge_ids(a, b)
                ia.append(a[ok])
                ib.append(b[ok])
                ij.append(ising.j[eid[ok]])
                im.append(np.full(int(ok.sum()), k, dtype=np.int64))
        nm = len(allm)
        self.mem = sp.csr_matrix((np.ones(len(rows), dtype=np.int64), (rows, cols)), shape=(n, nm))
        if ia:
            a, b, j, mm = (np.concatenate(x) for x in (ia, ib, ij, im))
        else:
            a = b = j = mm = np.zeros(0, dtype=np.int64)
        self.ia, self.ib, self.ij, self.im = a, b, j, mm
        self.inner = sp.csr_matrix((4 * j, (np.arange(len(a)), mm)), shape=(len(a), nm))
        self.flipmat = self.mem.T.tocsr()  # move -> members

    def deltas(self, ising: Ising, s: np.ndarray) -> np.ndarray:
        d = ising.flip_deltas(s)
        out = (self.mem.T @ d.T).T
        if len(self.ia):
            si = s.astype(np.int64)
            out = out + (self.inner.T @ (si[:, self.ia] * si[:, self.ib]).T).T
        return out


def steepest(ising: Ising, moves: MoveSet, s0: np.ndarray, max_iter: int = 1_000_000) -> tuple[np.ndarray, int]:
    """Steepest descent over the move set, one best move per read per step, until no move lowers the energy."""
    s = s0.astype(np.int8).copy()
    active = np.arange(len(s))
    steps = 0
    while len(active) and steps < max_iter:
        d = moves.deltas(ising, s[active])
        best = d.argmin(axis=1)
        gain = d[np.arange(len(active)), best]
        go = gain < 0
        if not go.any():
            break
        for r, m in zip(active[go], best[go], strict=True):
            idx = moves.flipmat.indices[moves.flipmat.indptr[m]:moves.flipmat.indptr[m + 1]]
            s[r, idx] = -s[r, idx]
        active = active[go]
        steps += 1
    return s, steps


def dwave_sd(bqm: dimod.BinaryQuadraticModel, s0: np.ndarray) -> np.ndarray:
    labels = list(range(s0.shape[1]))
    ss = SteepestDescentSolver().sample(bqm, initial_states=(s0.astype(np.int8), labels))
    return ss.record.sample[:, np.argsort(np.asarray(list(ss.variables)))]


def dwave_sa(bqm: dimod.BinaryQuadraticModel, s0: np.ndarray, sweeps: int, weakest: int, seed: int) -> np.ndarray:
    """Low-temperature anneal seeded from each read: start where the weakest uphill flip (2 x weakest) is
    accepted with probability 1/2, end where it is accepted with probability 1/1000."""
    labels = list(range(s0.shape[1]))
    b0, b1 = math.log(2) / (2 * weakest), math.log(1000) / (2 * weakest)
    ss = SimulatedAnnealingSampler().sample(
        bqm, initial_states=(s0.astype(np.int8), labels), initial_states_generator="none",
        num_reads=len(s0), num_sweeps=sweeps, beta_range=[b0, b1], beta_schedule_type="geometric", seed=seed)
    return ss.record.sample[:, np.argsort(np.asarray(list(ss.variables)))]


def timed(fn, *args, **kw):
    t0 = time.process_time()
    w0 = time.perf_counter()
    out = fn(*args, **kw)
    return out, time.process_time() - t0, time.perf_counter() - w0


_LIB = ctypes.CDLL(str(Path(__file__).resolve().parent / "libgreedy.so"))
_LIB.greedy_descent.restype = ctypes.c_int64


class GreedyC:
    """Flat arrays for greedy.c: adjacency, move members and internal edges grouped by move."""

    def __init__(self, ising: Ising, moves: MoveSet):
        adj = ising.jmat.tocsr()
        self.n = ising.n
        self.adj_ptr = adj.indptr.astype(np.int32)
        self.adj_idx = adj.indices.astype(np.int32)
        self.adj_j = adj.data.astype(np.int64)
        self.h = ising.h.astype(np.int64)
        fm = moves.flipmat
        self.n_moves = fm.shape[0]
        self.mv_ptr = fm.indptr.astype(np.int32)
        self.mv_idx = fm.indices.astype(np.int32)
        order = np.argsort(moves.im, kind="stable")
        self.in_a = moves.ia[order].astype(np.int32)
        self.in_b = moves.ib[order].astype(np.int32)
        self.in_j = moves.ij[order].astype(np.int64)
        self.in_ptr = np.searchsorted(moves.im[order], np.arange(self.n_moves + 1)).astype(np.int32)

    def descend(self, s0: np.ndarray) -> tuple[np.ndarray, int]:
        s = np.ascontiguousarray(s0, dtype=np.int8).copy()

        def ptr(x: np.ndarray) -> ctypes.c_void_p:
            return x.ctypes.data_as(ctypes.c_void_p)

        applied = _LIB.greedy_descent(
            ctypes.c_int32(self.n), ptr(self.adj_ptr), ptr(self.adj_idx), ptr(self.adj_j), ptr(self.h),
            ctypes.c_int32(self.n_moves), ptr(self.mv_ptr), ptr(self.mv_idx), ptr(self.in_ptr), ptr(self.in_a),
            ptr(self.in_b), ptr(self.in_j), ctypes.c_int32(len(s)), ptr(s))
        if applied < 0:
            raise MemoryError("greedy_descent could not allocate")
        return s, int(applied)


_LIB.loop_descent.restype = ctypes.c_int64


def ice_tiles(ising: Ising) -> tuple[np.ndarray, np.ndarray, np.ndarray, dict[str, int]]:
    """Gauged K4 tiles for the worm move: (tiles (T, 4), gauge (T, 4), mtile (n, 2)), and counts.

    A tile is a `tetrahedra` row whose six couplers exist and whose signs are gauge-equivalent to an
    antiferromagnetic K4 (J_ab sign = g_a g_b with g_0 = +1). Each spin keeps at most two tiles. The
    worm accepts on the exact energy change, so a dropped membership only narrows the proposals.
    """
    m = ising.model
    n = ising.n
    if "tetrahedra" not in m or not len(m["tetrahedra"]):
        return (np.zeros((0, 4), np.int32), np.zeros((0, 4), np.int32), np.full((n, 2), -1, np.int32),
                {"tiles": 0, "dropped": 0, "spins_over_two": 0})
    tet = m["tetrahedra"].astype(np.int64)
    pairs = list(itertools.combinations(range(4), 2))
    js = np.zeros((len(tet), 6), dtype=np.int64)
    ok = np.ones(len(tet), dtype=bool)
    for c, (a, b) in enumerate(pairs):
        eid, found = ising.edge_ids(tet[:, a], tet[:, b])
        js[:, c] = np.where(found, ising.j[eid], 0)
        ok &= found
    gauge = np.ones((len(tet), 4), dtype=np.int64)
    gauge[:, 1:] = np.sign(js[:, :3])
    for c, (a, b) in enumerate(pairs):
        ok &= np.sign(js[:, c]) == gauge[:, a] * gauge[:, b]
    tiles, gauge = tet[ok], gauge[ok]
    mtile = np.full((n, 2), -1, dtype=np.int64)
    fill = np.zeros(n, dtype=np.int64)
    over = 0
    for t in range(len(tiles)):
        for k in range(4):
            u = int(tiles[t, k])
            if fill[u] < 2:
                mtile[u, fill[u]] = t
                fill[u] += 1
            else:
                over += 1
    counts = {"tiles": int(len(tiles)), "dropped": int((~ok).sum()), "spins_over_two": over}
    return tiles.astype(np.int32), gauge.astype(np.int32), mtile.astype(np.int32), counts


class LoopC:
    """Worm descent of greedy.c on the gauged K4 tiles of an ice model."""

    def __init__(self, ising: Ising, adj: GreedyC):
        self.adj = adj
        self.tiles, self.gauge, self.mtile, self.counts = ice_tiles(ising)
        self.tiled = np.flatnonzero(self.mtile[:, 0] >= 0).astype(np.int32)

    def descend(self, s0: np.ndarray, zero_budget: int, seed: int, max_sweeps: int = 50,
                max_len: int = 8192) -> tuple[np.ndarray, int]:
        s = np.ascontiguousarray(s0, dtype=np.int8).copy()
        a = self.adj

        def ptr(x: np.ndarray) -> ctypes.c_void_p:
            return x.ctypes.data_as(ctypes.c_void_p)

        acc = _LIB.loop_descent(
            ctypes.c_int32(a.n), ptr(a.adj_ptr), ptr(a.adj_idx), ptr(a.adj_j), ptr(a.h), ptr(self.tiles),
            ptr(self.gauge), ptr(self.mtile), ctypes.c_int32(len(self.tiled)), ptr(self.tiled),
            ctypes.c_int32(len(s)), ptr(s), ctypes.c_int32(max_len), ctypes.c_int32(max_sweeps),
            ctypes.c_int64(zero_budget), ctypes.c_uint64(seed))
        if acc < 0:
            raise MemoryError("loop_descent could not allocate")
        return s, int(acc)


def nat_loop(ising: Ising, gnat: GreedyC, loop: LoopC, s0: np.ndarray, zero_budget: int, seed: int,
             rounds: int = 10) -> tuple[np.ndarray, int]:
    """Alternate natural-move descent and worm descent until the energies of all reads stop falling."""
    s, _ = gnat.descend(s0)
    e = ising.energies(s)
    done = 0
    for done in range(1, rounds + 1):
        s, _ = loop.descend(s, zero_budget, seed + done)
        s, _ = gnat.descend(s)
        e_new = ising.energies(s)
        if np.array_equal(e_new, e):
            break
        e = e_new
    return s, done
