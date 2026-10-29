"""Pair-aware moves for twisted ice with planted pairs: ctypes wrappers over pairworm.c."""
from __future__ import annotations

import ctypes
from pathlib import Path

import numpy as np

import descent as D

_LIB = ctypes.CDLL(str(Path(__file__).resolve().parent / "libpairworm.so"))
_LIB.pair_worm_descent.restype = ctypes.c_int64
_LIB.defect_counts.restype = None
_LIB.crossover.restype = ctypes.c_int64
STRONG_MILLI = 500  # tile and pair couplers are at or above this; the glass is below


def _ptr(x: np.ndarray) -> ctypes.c_void_p:
    return x.ctypes.data_as(ctypes.c_void_p)


class PairWorm:
    """Tiles, gauges and pair partners of a model, and the pair-aware worm descent over them."""

    def __init__(self, ising: D.Ising, adj: D.GreedyC):
        self.adj = adj
        self.tiles, self.gauge, self.mtile, self.counts = D.ice_tiles(ising)
        self.tiled = np.flatnonzero(self.mtile[:, 0] >= 0).astype(np.int32)
        m = ising.model
        pairs = m["pairs"].astype(np.int64) if "pairs" in m and len(m["pairs"]) else np.zeros((0, 2), np.int64)
        eid, ok = ising.edge_ids(pairs[:, 0], pairs[:, 1])
        if not ok.all() or (ising.j[eid] > 0).any():
            raise ValueError("every pair must be an existing ferromagnetic coupler")
        self.pairs = np.ascontiguousarray(pairs, dtype=np.int32)
        partner = np.full(ising.n, -1, dtype=np.int32)
        partner[pairs[:, 0]] = pairs[:, 1]
        partner[pairs[:, 1]] = pairs[:, 0]
        self.partner = partner
        self.counts["pairs"] = int(len(pairs))
        strong = ising.jmat.copy()
        strong.data[np.abs(strong.data) < STRONG_MILLI] = 0
        strong.eliminate_zeros()
        strong = strong.tocsr()
        self.sadj_ptr = strong.indptr.astype(np.int32)
        self.sadj_idx = strong.indices.astype(np.int32)

    def descend(self, s0: np.ndarray, zero_budget: int, seed: int, max_sweeps: int = 50, max_len: int = 32,
                random_pass: bool = True, defect_depth: int = 0, random_depth: int = 0,
                prune: int = 6000, temp: float = 0.0) -> tuple[np.ndarray, int]:
        s = np.ascontiguousarray(s0, dtype=np.int8).copy()
        a = self.adj
        acc = _LIB.pair_worm_descent(
            ctypes.c_int32(a.n), _ptr(a.adj_ptr), _ptr(a.adj_idx), _ptr(a.adj_j), _ptr(a.h), _ptr(self.tiles),
            _ptr(self.gauge), _ptr(self.mtile), ctypes.c_int32(len(self.tiles)), _ptr(self.partner),
            ctypes.c_int32(len(self.tiled)), _ptr(self.tiled), _ptr(self.pairs), ctypes.c_int32(len(self.pairs)),
            ctypes.c_int32(len(s)), _ptr(s), ctypes.c_int32(max_len), ctypes.c_int32(max_sweeps),
            ctypes.c_int64(zero_budget), ctypes.c_int(int(random_pass)), ctypes.c_int(defect_depth),
            ctypes.c_int(random_depth), ctypes.c_int64(prune), ctypes.c_double(temp), ctypes.c_uint64(seed))
        if acc < 0:
            raise MemoryError("pair_worm_descent could not allocate")
        return s, int(acc)

    def crossover(self, s: np.ndarray, pool: np.ndarray, align: bool = True, min_size: int = 1,
                  max_size: int = 1 << 30) -> tuple[np.ndarray, int]:
        """Flip into s every strong-connected block on which a pool state differs, when that lowers the energy."""
        s = np.ascontiguousarray(s, dtype=np.int8).copy()
        pool = np.ascontiguousarray(pool, dtype=np.int8)
        a = self.adj
        k = _LIB.crossover(ctypes.c_int32(a.n), _ptr(a.adj_ptr), _ptr(a.adj_idx), _ptr(a.adj_j), _ptr(a.h),
                           _ptr(self.sadj_ptr), _ptr(self.sadj_idx), _ptr(s), ctypes.c_int32(len(pool)), _ptr(pool),
                           ctypes.c_int(int(align)), ctypes.c_int32(min_size), ctypes.c_int32(max_size))
        if k < 0:
            raise MemoryError("crossover could not allocate")
        return s, int(k)

    def defects(self, s: np.ndarray) -> np.ndarray:
        """(reads, 2): charged tiles and broken pairs per read."""
        s = np.ascontiguousarray(s, dtype=np.int8)
        out = np.zeros((len(s), 2), dtype=np.int32)
        _LIB.defect_counts(ctypes.c_int32(self.adj.n), _ptr(self.tiles), _ptr(self.gauge),
                           ctypes.c_int32(len(self.tiles)), _ptr(self.pairs), ctypes.c_int32(len(self.pairs)),
                           ctypes.c_int32(len(s)), _ptr(s), _ptr(out))
        return out
