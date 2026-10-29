"""The ice-tiles postprocess: worm anneal from the best reads inside a CPU budget, then worm descent.

A spec string selects the variant: `name:key=value,...`. `name` is `anneal` (worm Metropolis with a
geometric beta schedule, then zero-temperature worm descent for the rest of the budget) or `desc`
(zero-temperature worm descent only). Keys: b0 and b1 scale the schedule ends ln2/(2w) and
ln1000/(2w), where w is the weakest nonzero coupling; frac is the anneal share of the read's budget;
tiles and pairs add whole-tile and strong-pair seeds (0/1); tw repeats the tile seeds; cross is the
probability of crossing a rigid tile instead of ending the leg there; guide scales the leg-choice beta; reads is the number of reads, best
first, that share the budget evenly; fe is the free-spin sweep period in worms; len the worm cap;
pw flips every spin together with its rigid partner (strong pair outside the tiles) during the walk;
sa makes the periodic Metropolis sweep visit every spin (a single-flip anneal between worms) instead
of the untiled spins only; nat48 descends all 48 reads first and starts from the lowest descended read.
Every read gets natural-move descent before the worm and after it (all inside the budget).
"""
from __future__ import annotations

import math
import time

import numpy as np

import descent as D
import icetiles as IT

DEFAULT_SPEC = "anneal"


def parse(spec: str) -> tuple[str, dict[str, float]]:
    name, _, rest = spec.partition(":")
    kw = {"b0": 1.0, "b1": 1.0, "frac": 0.8, "tiles": 0, "pairs": 0, "tw": 1, "cross": 0.0, "guide": 1.0, "reads": 1,
          "fe": 16, "len": 4096, "pw": 0, "sa": 0, "nat48": 0}
    for item in filter(None, rest.split(",")):
        k, _, v = item.partition("=")
        if k not in kw:
            raise ValueError(f"unknown key {k} in spec {spec}")
        kw[k] = float(v)
    if name not in ("anneal", "desc"):
        raise ValueError(f"unknown method {name}")
    return name, kw


class Prepared:
    """Model-only preparation that can run while the QPU call is in flight."""

    def __init__(self, model: dict[str, np.ndarray], spec: str):
        self.ising = D.Ising(model)
        moves, _ = D.natural_moves(self.ising)
        self.gnat = D.GreedyC(self.ising, D.MoveSet(self.ising, moves))
        self.tt = IT.TileTables(self.ising)
        self.name, self.kw = parse(spec)
        kw = self.kw
        self.worm = IT.WormC(self.gnat, self.tt, *self.tt.seeds(tiles=bool(kw["tiles"]), pairs=bool(kw["pairs"]),
                                                                 tile_weight=int(kw["tw"])))
        w = self.ising.weakest()
        self.b_lo = kw["b0"] * math.log(2) / (2 * w)
        self.b_hi = kw["b1"] * math.log(1000) / (2 * w)


def run(prep: Prepared, reads: np.ndarray, budget: float, seed: int) -> tuple[np.ndarray, int, int, dict]:
    """Postprocess the 48 reads of one call. Returns (best state, best energy, reads used, stats)."""
    ising, gnat, wc, kw, name = prep.ising, prep.gnat, prep.worm, prep.kw, prep.name
    t_start = time.process_time()
    if kw["nat48"]:
        reads, _ = gnat.descend(reads)
    energies = ising.energies(reads)
    order = np.argsort(energies, kind="stable")
    n_reads = max(1, min(int(kw["reads"]), len(reads)))
    per = budget / n_reads
    best_e, sb, stats = None, None, {}
    used = 0
    for r in range(n_reads):
        i = int(order[r])
        if kw["nat48"]:
            s1, e1 = reads[i:i + 1], int(energies[i])
        else:
            s1, _ = gnat.descend(reads[i:i + 1])
            e1 = int(ising.energies(s1)[0])
        left = per * (r + 1) - (time.process_time() - t_start)
        if r > 0 and left <= 0.0:
            break
        used += 1
        common = {"p_cross": kw["cross"], "pair_walk": bool(kw["pw"]), "sweep_all": bool(kw["sa"]), "free_every": int(kw["fe"]), "max_len": int(kw["len"])}
        if name == "desc":
            _, sb_r, _, e_r, st = wc.run(s1[0], e1, max(left, 0.0), seed + r, zero_budget=4 * ising.n, **common)
        else:
            sf, sb_r, ef, e_r, st = wc.run(s1[0], e1, kw["frac"] * max(left, 0.0), seed + r, beta0=prep.b_lo,
                                           beta1=prep.b_hi, guide=kw["guide"], **common)
            tail = per * (r + 1) - (time.process_time() - t_start)
            if tail > 0.002:
                _, sb2, _, e2, st2 = wc.run(sf, ef, tail, seed + 1000 + r, zero_budget=4 * ising.n, **common)
                st["tail_improved"] = st2["improved"]
                if e2 < e_r:
                    e_r, sb_r = e2, sb2
        sp, _ = gnat.descend(sb_r[None, :])
        e_p = int(ising.energies(sp)[0])
        st["polish"] = e_r - e_p
        if best_e is None or e_p < best_e:
            best_e, sb, stats = e_p, sp[0], st
    assert sb is not None and best_e is not None
    return sb, best_e, used, stats
