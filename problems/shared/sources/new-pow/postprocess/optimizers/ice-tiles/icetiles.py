"""Tile tables and the ctypes wrapper of icetiles.c for the ice-tiles optimizer.

Every `tetrahedra` row is a tile. Its 16-entry energy table comes from the model couplers inside the
tile (the full coupler value, also on an edge that two overlapping tiles share), so a reversed-bond
defect tile, a K4 tile with missing couplers and an overlapping tile all get their own ground set.
"""
from __future__ import annotations

import ctypes
import itertools
from pathlib import Path

import numpy as np

import descent as D

_LIB = ctypes.CDLL(str(Path(__file__).resolve().parent / "libicetiles.so"))
_LIB.worm_run.restype = ctypes.c_int

STRONG_PAIR_MIN = 400  # a coupler outside every tile at or above this magnitude is a pair seed


def _ptr(x: np.ndarray) -> ctypes.c_void_p:
    return x.ctypes.data_as(ctypes.c_void_p)


class TileTables:
    """Tiles, per-tile energy tables, spin-to-tile lists, free spins and the seed sets."""

    def __init__(self, ising: D.Ising):
        m = ising.model
        n = ising.n
        tet = m["tetrahedra"].astype(np.int64) if "tetrahedra" in m else np.zeros((0, 4), np.int64)
        t_count = len(tet)
        pairs = list(itertools.combinations(range(4), 2))
        share = np.zeros(len(ising.j), dtype=np.int64)
        eids = np.zeros((t_count, 6), dtype=np.int64)
        found = np.zeros((t_count, 6), dtype=bool)
        for c, (a, b) in enumerate(pairs):
            eid, ok = ising.edge_ids(tet[:, a], tet[:, b])
            eids[:, c], found[:, c] = eid, ok
            np.add.at(share, eid[ok], 1)
        jt = np.where(found, ising.j[eids], 0).astype(np.int64)
        # energy table: bit k set when corner k is +1
        states = np.array([[1 if (idx >> k) & 1 else -1 for k in range(4)] for idx in range(16)], dtype=np.int64)
        prod = np.stack([states[:, a] * states[:, b] for a, b in pairs], axis=1)  # 16 x 6
        etab = jt @ prod.T  # T x 16
        emin = etab.min(axis=1, keepdims=True) if t_count else np.zeros((0, 1), np.int64)
        etab = etab - emin
        self.tiles = np.ascontiguousarray(tet, dtype=np.int32)
        self.etab = np.ascontiguousarray(etab, dtype=np.int64)
        self.shared_edges = int((share > 1).sum())
        # spin -> tiles CSR
        if t_count:
            spin = tet.ravel()
            tile = np.repeat(np.arange(t_count), 4)
            order = np.argsort(spin, kind="stable")
            counts = np.bincount(spin, minlength=n)
            self.st_ptr = np.concatenate([[0], np.cumsum(counts)]).astype(np.int32)
            self.st_idx = tile[order].astype(np.int32)
        else:
            self.st_ptr = np.zeros(n + 1, dtype=np.int32)
            self.st_idx = np.zeros(0, dtype=np.int32)
        tiled_mask = (self.st_ptr[1:] - self.st_ptr[:-1]) > 0
        self.tiled = np.flatnonzero(tiled_mask).astype(np.int32)
        self.free = np.flatnonzero(~tiled_mask).astype(np.int32)
        self.all_spins = np.arange(n, dtype=np.int32)
        # strong pairs outside every tile
        in_tile = np.zeros(len(ising.j), dtype=bool)
        in_tile[eids[found]] = True
        strong = (np.abs(ising.j) >= STRONG_PAIR_MIN) & ~in_tile
        self.strong_pairs = ising.edges[strong].astype(np.int32)
        self.strong_j = ising.j[strong].astype(np.int64)
        # rigid partner map: a spin with exactly one strong pair outside the tiles
        deg = np.bincount(self.strong_pairs.ravel(), minlength=n) if len(self.strong_pairs) else np.zeros(n, np.int64)
        self.partner = np.full(n, -1, dtype=np.int32)
        self.partner_j = np.zeros(n, dtype=np.int64)
        for (u, v), j in zip(self.strong_pairs.tolist(), self.strong_j.tolist(), strict=True):
            if deg[u] == 1 and deg[v] == 1:
                self.partner[u], self.partner[v] = v, u
                self.partner_j[u] = self.partner_j[v] = j
        self.no_partner = np.full(n, -1, dtype=np.int32)
        rigid = int(((self.etab == 0).sum(axis=1) == 2).sum()) if t_count else 0
        self.counts = {"tiles": int(t_count), "rigid_tiles": rigid, "shared_tile_edges": self.shared_edges,
                       "tiled_spins": int(len(self.tiled)), "free_spins": int(len(self.free)),
                       "strong_pairs": int(len(self.strong_pairs)), "partnered": int((self.partner >= 0).sum()),
                       "max_tiles_per_spin": int((self.st_ptr[1:] - self.st_ptr[:-1]).max()) if t_count else 0}

    def seeds(self, singles: bool = True, tiles: bool = False, pairs: bool = False,
              tile_weight: int = 1) -> tuple[np.ndarray, np.ndarray]:
        """CSR of seed spin sets: tiled single spins, whole tiles (repeated `tile_weight` times), strong pairs."""
        parts: list[np.ndarray] = []
        if singles:
            parts.append(self.tiled.reshape(-1, 1).astype(np.int32))
        if tiles and len(self.tiles):
            parts.extend([self.tiles] * tile_weight)
        if pairs and len(self.strong_pairs):
            parts.append(self.strong_pairs)
        if not parts:
            parts.append(self.tiled.reshape(-1, 1).astype(np.int32))
        lens = np.concatenate([np.full(len(p), p.shape[1], dtype=np.int64) for p in parts])
        ptr = np.concatenate([[0], np.cumsum(lens)]).astype(np.int32)
        idx = np.concatenate([p.ravel() for p in parts]).astype(np.int32)
        return ptr, idx


class WormC:
    """One worm run on one state within a CPU budget. Returns (final state, best state, final e, best e, stats)."""

    def __init__(self, adj: D.GreedyC, tt: TileTables, seed_ptr: np.ndarray, seed_idx: np.ndarray):
        self.adj = adj
        self.tt = tt
        self.seed_ptr = np.ascontiguousarray(seed_ptr, dtype=np.int32)
        self.seed_idx = np.ascontiguousarray(seed_idx, dtype=np.int32)

    def run(self, s0: np.ndarray, e0: int, budget_s: float, seed: int, beta0: float = 0.0, beta1: float = 0.0,
            guide: float = 1.0, p_cross: float = 0.0, max_len: int = 4096, zero_budget: int = 0,
            free_every: int = 16, max_worms: int = 1 << 40, patience: int = 0, pair_walk: bool = False,
            sweep_all: bool = False) -> tuple[np.ndarray, np.ndarray, int, int, dict[str, int]]:
        """`sweep_all` makes the periodic Metropolis sweep visit every spin instead of the untiled ones."""
        a, tt = self.adj, self.tt
        s = np.ascontiguousarray(s0, dtype=np.int8).copy()
        best = np.zeros_like(s)
        e_io = np.array([e0, e0], dtype=np.int64)
        stats = np.zeros(6, dtype=np.int64)
        partner = tt.partner if pair_walk else tt.no_partner
        sweep = tt.all_spins if sweep_all else tt.free
        rc = _LIB.worm_run(
            ctypes.c_int32(a.n), _ptr(a.adj_ptr), _ptr(a.adj_idx), _ptr(a.adj_j), _ptr(a.h),
            ctypes.c_int32(len(tt.tiles)), _ptr(tt.tiles), _ptr(tt.etab), _ptr(tt.st_ptr),
            _ptr(tt.st_idx), _ptr(partner), _ptr(tt.partner_j), ctypes.c_int32(len(self.seed_ptr) - 1),
            _ptr(self.seed_ptr), _ptr(self.seed_idx),
            ctypes.c_int32(len(sweep)), _ptr(sweep), _ptr(s), _ptr(best), _ptr(e_io), ctypes.c_double(budget_s),
            ctypes.c_int64(max_worms), ctypes.c_int64(patience), ctypes.c_double(beta0), ctypes.c_double(beta1),
            ctypes.c_double(guide), ctypes.c_double(p_cross), ctypes.c_int32(max_len),
            ctypes.c_int64(zero_budget), ctypes.c_int32(free_every), ctypes.c_uint64(seed), _ptr(stats))
        if rc != 0:
            raise MemoryError("worm_run could not allocate")
        names = ("worms", "accepted", "improved", "failed", "crossed", "free_flips")
        return s, best, int(e_io[0]), int(e_io[1]), dict(zip(names, stats.tolist(), strict=True))
