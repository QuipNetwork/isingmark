#!/usr/bin/env python3
"""Extend new-pow cells from 12 (or 24) models to 100 models.

Usage:
  gen100.py check CELL...     regenerate the saved models of each cell and compare them
  gen100.py extend CELL...    check, then write the new models, the capture request and
                              the cell record under models-100/<cell>/

A cell is extended only when its generator reproduces every saved model array for array.
Nonce i for i >= 13 is the lowercase hex of SHA256(b"new-pow/models-100/" + str(i)).
A rejected nonce is recorded and the next index above 100 replaces it.
Each run of ``extend`` writes models-100/<cell>/record.json; build_manifest.py collects them.
"""

from __future__ import annotations

import hashlib
import importlib.util
import io
import json
import sys
import time
import traceback
from functools import lru_cache
from pathlib import Path
from typing import Callable

import os

for _name in ("OMP_NUM_THREADS", "OPENBLAS_NUM_THREADS", "MKL_NUM_THREADS", "RAYON_NUM_THREADS", "NUMBA_NUM_THREADS"):
    os.environ[_name] = "1"

import numpy as np  # noqa: E402

HERE = Path(__file__).resolve().parent
NP = HERE.parent
REGIMES = NP.parent
SUITE = REGIMES / "qpu-suite"
F17 = REGIMES / "qpu-f17"
FAMILIES = NP / "families"
SPEC = REGIMES / "topology-live-2026-09-21.spec.json"
TARGET = 100

sys.path.insert(0, str(NP))
sys.path.insert(0, str(SUITE))
sys.path.insert(0, str(F17))


class Rejected(Exception):
    """The generator refused the nonce (admission failure)."""


def nonce_for(i: int) -> str:
    return hashlib.sha256(b"new-pow/models-100/" + str(i).encode()).hexdigest()


def stamp() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def load_npz(path: Path) -> dict[str, np.ndarray]:
    with np.load(path, allow_pickle=False) as data:
        return {k: data[k] for k in data.files}


def load_file_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


# ---------------------------------------------------------------- suite generators


def suite(module: str):
    import importlib

    return importlib.import_module(f"work.{module}")


@lru_cache(maxsize=None)
def twisted_base(nonce: str) -> dict[str, np.ndarray]:
    """The suite ice-twisted model, the base of every f17 twisted variant."""
    saved = SUITE / "work/cells/ice-twisted/models" / f"{nonce}.npz"
    if saved.exists():
        return load_npz(saved)
    return suite("ice_twisted").generate_model(nonce)


@lru_cache(maxsize=None)
def glass_r8_base(nonce: str) -> dict[str, np.ndarray]:
    saved = SUITE / "work/cells/ice-glass-r8/models" / f"{nonce}.npz"
    if saved.exists():
        return load_npz(saved)
    return suite("ice").generate_model(nonce, 125)


def cbfm_suite(nonce: str) -> dict:
    parameters = json.loads((SUITE / "work/cells/cbfm-p/parameters.json").read_text())
    return suite("cbfm").generate_model(nonce, parameters)


SUITE_CELLS: dict[str, tuple[str, Callable[[str], dict]]] = {
    "ice-twisted": ("qpu-suite/work/ice_twisted.py generate_model(nonce)", lambda n: suite("ice_twisted").generate_model(n)),
    "ice-fields-h500": ("qpu-suite/work/ice_fields.py generate_model(nonce, 500)", lambda n: suite("ice_fields").generate_model(n, 500)),
    "ice-fields-h1000": ("qpu-suite/work/ice_fields.py generate_model(nonce, 1000)", lambda n: suite("ice_fields").generate_model(n, 1000)),
    "ice-glass-r5": ("qpu-suite/work/ice.py generate_model(nonce, 200)", lambda n: suite("ice").generate_model(n, 200)),
    "ice-glass-r8": ("qpu-suite/work/ice.py generate_model(nonce, 125)", lambda n: suite("ice").generate_model(n, 125)),
    "ice-defect-p50": ("qpu-suite/work/ice_defect.py generate_model(nonce, 2)", lambda n: suite("ice_defect").generate_model(n, 2)),
    "ice-defect-p25": ("qpu-suite/work/ice_defect.py generate_model(nonce, 4)", lambda n: suite("ice_defect").generate_model(n, 4)),
    "ice-defect-p125": ("qpu-suite/work/ice_sparse_defect.py generate_model(nonce, 8)", lambda n: suite("ice_sparse_defect").generate_model(n, 8)),
    "ice-defect-p0625": ("qpu-suite/work/ice_sparse_defect.py generate_model(nonce, 16)", lambda n: suite("ice_sparse_defect").generate_model(n, 16)),
    "ice-defect-p03125": ("qpu-suite/work/ice_rare_defect.py generate_model(nonce, 32)", lambda n: suite("ice_rare_defect").generate_model(n, 32)),
    "ice-defect-p015625": ("qpu-suite/work/ice_rare_defect.py generate_model(nonce, 64)", lambda n: suite("ice_rare_defect").generate_model(n, 64)),
    "ice-overlap-c2": ("qpu-suite/work/ice_overlap.py generate_model(nonce, 2)", lambda n: suite("ice_overlap").generate_model(n, 2)),
    "ice-overlap-c3": ("qpu-suite/work/ice_overlap.py generate_model(nonce, 3)", lambda n: suite("ice_overlap").generate_model(n, 3)),
    "ice-planted": ("qpu-suite/work/ice_planted.py generate_model(nonce)", lambda n: suite("ice_planted").generate_model(n)),
    "ice-three-level": ("qpu-suite/work/ice_three_level.py generate_model(nonce)", lambda n: suite("ice_three_level").generate_model(n)),
    "kagome-net": ("qpu-suite/work/kagome.py generate_model(nonce)", lambda n: suite("kagome").generate_model(n)),
    "strong-net-d1.5": ("qpu-suite/work/strongnet.py generate_model(nonce, 1.5)", lambda n: suite("strongnet").generate_model(n, 1.5)),
    "strong-net-d2": ("qpu-suite/work/strongnet.py generate_model(nonce, 2)", lambda n: suite("strongnet").generate_model(n, 2)),
    "strong-net-d3": ("qpu-suite/work/strongnet.py generate_model(nonce, 3)", lambda n: suite("strongnet").generate_model(n, 3)),
    "tri-match-r4": ("qpu-suite/work/trimatch.py generate_model(nonce, 4)", lambda n: suite("trimatch").generate_model(n, 4)),
    "tri-match-r6": ("qpu-suite/work/trimatch.py generate_model(nonce, 6)", lambda n: suite("trimatch").generate_model(n, 6)),
    "tri-match-power": ("qpu-suite/work/trimatch_power.py generate_model(nonce)", lambda n: suite("trimatch_power").generate_model(n)),
    "dimer-r5": ("qpu-suite/work/dimer_crossover.py generate_model(nonce, 5)", lambda n: suite("dimer_crossover").generate_model(n, 5)),
    "dimer-r6": ("qpu-suite/work/dimer_crossover.py generate_model(nonce, 6)", lambda n: suite("dimer_crossover").generate_model(n, 6)),
    "dimer-r7": ("qpu-suite/work/dimer_crossover.py generate_model(nonce, 7)", lambda n: suite("dimer_crossover").generate_model(n, 7)),
    "dimer-power-mixed": ("qpu-suite/work/mixed_power.py generate_model(nonce)", lambda n: suite("mixed_power").generate_model(n)),
    "cubic-no-dimer": ("qpu-suite/work/cubic.py generate_model(nonce)", lambda n: suite("cubic").generate_model(n)),
    "diamond-3d": ("qpu-suite/work/diamond.py generate_model(nonce)", lambda n: suite("diamond").generate_model(n)),
    "cbfm-p": ("qpu-suite/work/cbfm.py generate_model(nonce, cells/cbfm-p/parameters.json)", cbfm_suite),
    "frust-dense-k12": ("qpu-suite/work/dense_blocks.py generate_model(nonce, 12)", lambda n: suite("dense_blocks").generate_model(n, 12)),
    "frust-dense-k16": ("qpu-suite/work/dense_blocks.py generate_model(nonce, 16)", lambda n: suite("dense_blocks").generate_model(n, 16)),
    "pair-loop": ("qpu-suite/work/pair_loop.py generate_model(nonce)", lambda n: suite("pair_loop").generate_model(n)),
    "strip-glass": ("qpu-suite/work/strip_glass.py generate_model(nonce)", lambda n: suite("strip_glass").generate_model(n)),
    "wsc": ("qpu-suite/work/wsc.py generate_model(nonce)", lambda n: suite("wsc").generate_model(n)),
    "dcl-k44-l5": ("qpu-suite/work/dcl.py generate_model(nonce, 200)", lambda n: suite("dcl").generate_model(n, 200)),
    "dcl-k44-l7": ("qpu-suite/work/dcl.py generate_model(nonce, 143)", lambda n: suite("dcl").generate_model(n, 143)),
    "fcl-planted-a0.2": ("qpu-suite/work/fcl.py generate_model(nonce, 2)", lambda n: suite("fcl").generate_model(n, 2)),
    "fcl-planted-a0.3": ("qpu-suite/work/fcl.py generate_model(nonce, 3)", lambda n: suite("fcl").generate_model(n, 3)),
}

# ---------------------------------------------------------------- f17 generators
# The f17 scripts build whole cells in one main(). These wrappers repeat each main's per-nonce
# body with the script's own helpers and the same saved key order.


def f17(module: str):
    import importlib

    return importlib.import_module(module)


def variant(base: Callable[[str], dict], transform: Callable[[dict, str], dict]) -> Callable[[str], dict]:
    """generate2.py main: transform the base model, keep five arrays and the pairs."""

    def build(nonce: str) -> dict:
        out = transform(dict(base(nonce)), nonce)
        keep = {k: out[k] for k in ("edges", "j_milli", "h_milli", "physical_nodes", "tetrahedra")}
        if "pairs" in out:
            keep["pairs"] = out["pairs"]
        return keep

    return build


def wave(pairs: int = 0, weak: int = 62, pair_milli: int = -1000, loops: int = 0, loop_length: int = 4):
    """wave_run.py build (and generate6.py, the same body): pairs or loops, pair strength, weak scale."""

    def build(nonce: str) -> dict:
        g2, wr = f17("generate2"), f17("wave_run")
        model = dict(twisted_base(nonce))
        if loops:
            model = g2.add_loops(model, nonce, loops, loop_length)
        elif pairs:
            model = g2.add_dimers(model, nonce, pairs)
        else:
            model = {**model, "pairs": np.zeros((0, 2), dtype=np.int64)}
        if pair_milli != -1000:
            model = wr.set_pair_strength(model, pair_milli)
        model = g2.rescale_weak(model, weak)
        keys = ("edges", "j_milli", "h_milli", "physical_nodes", "tetrahedra", "pairs", "loop_edges")
        return {k: model[k] for k in keys if k in model}

    return build


def twisted_window(side: int) -> Callable[[str], dict]:
    """window.py main: cut the suite ice-twisted model to a square window."""
    return lambda nonce: f17("window").cut(dict(twisted_base(nonce)), side)


def dimer1200_window(side: int) -> Callable[[str], dict]:
    """generate7.py main: cut ice-twisted-w62-dimer1200 to a window, keep the pairs inside it."""
    full = wave(pairs=1200, weak=62)

    def build(nonce: str) -> dict:
        window = f17("window")
        model = full(nonce)
        sub = window.cut(model, side)
        kept = np.zeros(window.N_SPINS, dtype=bool)
        kept[sub["kept"]] = True
        sub["pairs"] = model["pairs"][kept[model["pairs"]].all(axis=1)]
        return sub

    return build


@lru_cache(maxsize=1)
def k4_graph():
    g3 = f17("generate3")
    nodes, edges = g3.load_graph()
    edge_index = {(int(a), int(b)): i for i, (a, b) in enumerate(edges.tolist())}
    return nodes, edges, edge_index, g3.enumerate_k4(edges)


def k4_cell(cell: str) -> Callable[[str], dict]:
    """generate3.py main for ice-overlap2, ice-all-k4 and ice-charge."""

    def build(nonce: str) -> dict:
        g3 = f17("generate3")
        nodes, edges, edge_index, k4 = k4_graph()
        rng = g3.rng_for(nonce, cell)
        if cell == "ice-overlap2":
            tiles = g3.select_overlap(k4, edge_index, rng, edge_cap=2, spin_cap=3)
            scale, background = 500, 31
            charges = np.zeros(len(tiles), dtype=np.int64)
        elif cell == "ice-all-k4":
            tiles = k4
            scale, background = 250, 31
            charges = np.zeros(len(tiles), dtype=np.int64)
        else:
            tiles = twisted_base(nonce)["tetrahedra"]
            scale, background = 250, 31
            charges = rng.choice(np.asarray([0, 2, -2]), size=len(tiles), p=[0.5, 0.25, 0.25])
        gauges = rng.choice(np.asarray([-1, 1], dtype=np.int64), size=(len(tiles), 4))
        j, h = g3.build(edges, edge_index, tiles, gauges, charges, scale, background, rng)
        keep = j != 0
        return {"edges": edges[keep], "j_milli": j[keep], "h_milli": h, "physical_nodes": nodes,
                "tetrahedra": tiles, "local_gauges": gauges.astype(np.int8), "charges": charges}

    return build


def parity_k44(nonce: str) -> dict:
    """generate5.py main, one nonce."""
    from collections import defaultdict

    g3, g5 = f17("generate3"), f17("generate5")
    nodes, edges = g3.load_graph()
    edge_index = {(int(a), int(b)): i for i, (a, b) in enumerate(edges.tolist())}
    group = g5.groups(nodes)
    vertical = [g for g in group if g[0] == 0 and len(group[g]) == 4]
    horizontal = [g for g in group if g[0] == 1 and len(group[g]) == 4]
    candidates: dict[tuple, list[tuple]] = defaultdict(list)
    for v in vertical:
        for hz in horizontal:
            if all((min(a, b), max(a, b)) in edge_index for a in group[v] for b in group[hz]):
                candidates[v].append(hz)
    rng = np.random.default_rng(int.from_bytes(hashlib.sha256(b"parity-k44" + bytes.fromhex(nonce)).digest()[:8], "little"))
    use: dict = defaultdict(int)
    gadgets = []
    for v in rng.permutation(len(vertical)):
        v = vertical[v]
        options = [hz for hz in candidates[v] if use[hz] == 1] or [hz for hz in candidates[v] if use[hz] == 0]
        if not options:
            continue
        hz = options[rng.integers(len(options))]
        use[hz] += 1
        gadgets.append((v, hz))
    j = np.zeros(len(edges), dtype=np.int64)
    gauges = []
    for v, hz in gadgets:
        g = rng.choice(np.asarray([-1, 1]), size=4)
        gauges.append(g)
        for a, aux in enumerate(group[v]):
            for i, data in enumerate(group[hz]):
                k = edge_index[(min(aux, data), max(aux, data))]
                j[k] = -g5.J_GADGET * (1 - 2 * int(a == i)) * int(g[i])
    data_spins = np.zeros(g5.N, dtype=bool)
    for _, hz in gadgets:
        data_spins[group[hz]] = True
    for k, (a, b) in enumerate(edges.tolist()):
        if j[k] == 0 and data_spins[a] and data_spins[b]:
            j[k] = int(rng.choice([-g5.SELECTOR, g5.SELECTOR]))
    keep = j != 0
    return {"edges": edges[keep], "j_milli": j[keep], "h_milli": np.zeros(g5.N, dtype=np.int64),
            "physical_nodes": nodes,
            "gadgets": np.asarray([group[v] + group[hz] for v, hz in gadgets], dtype=np.int64),
            "gauges": np.asarray(gauges, dtype=np.int8)}


@lru_cache(maxsize=1)
def native_base():
    from quip_miner_dwave import regimes, replay

    base = regimes.native_cell("native-pm1", replay.load_spec(str(SPEC)))
    edges = np.sort(np.asarray(base.draw_edges, dtype=np.int64), axis=1)
    return edges, np.asarray(base.physical_nodes, dtype=np.int64), base.n_spins


def cbfm_f17(name: str) -> Callable[[str], dict]:
    """qpu-f17/generate.py main for the cbfm cells."""

    def build(nonce: str) -> dict:
        gen = load_file_module("f17_generate", F17 / "generate.py") if "f17_generate" not in sys.modules else sys.modules["f17_generate"]
        edges, physical, n_spins = native_base()
        return {"physical_nodes": physical, **gen.generate(name, nonce, edges, n_spins)}

    return build


def _g2(name: str):
    return lambda model, nonce: getattr(f17("generate2"), name)(model, nonce)


F17_CELLS: dict[str, tuple[str, Callable[[str], dict]]] = {
    "ice-twisted-w62": ("qpu-f17/generate2.py rescale_weak(ice-twisted, 62)", variant(twisted_base, lambda m, n: f17("generate2").rescale_weak(m, 62))),
    "ice-twisted-w250": ("qpu-f17/generate2.py rescale_weak(ice-twisted, 250)", variant(twisted_base, lambda m, n: f17("generate2").rescale_weak(m, 250))),
    "ice-twisted-w31": ("qpu-f17/generate2.py rescale_weak(ice-twisted, 31)", variant(twisted_base, lambda m, n: f17("generate2").rescale_weak(m, 31))),
    "ice-twisted-w45": ("qpu-f17/generate2.py rescale_weak(ice-twisted, 45)", variant(twisted_base, lambda m, n: f17("generate2").rescale_weak(m, 45))),
    "ice-twisted-w90": ("qpu-f17/generate2.py rescale_weak(ice-twisted, 90)", variant(twisted_base, lambda m, n: f17("generate2").rescale_weak(m, 90))),
    "ice-twisted-dimer": ("qpu-f17/generate2.py add_dimers(ice-twisted, 600)", variant(twisted_base, lambda m, n: f17("generate2").add_dimers(m, n, 600))),
    "ice-twisted-dimer1200": ("qpu-f17/generate2.py add_dimers(ice-twisted, 1200)", variant(twisted_base, lambda m, n: f17("generate2").add_dimers(m, n, 1200))),
    "ice-random-k4": ("qpu-f17/generate2.py random_k4(ice-twisted)", variant(twisted_base, _g2("random_k4"))),
    "ice-glass-r16": ("qpu-f17/generate2.py rescale_weak(ice-glass-r8, 62)", variant(glass_r8_base, lambda m, n: f17("generate2").rescale_weak(m, 62))),
    "ice-twisted-w62-dimer": ("qpu-f17/generate2.py rescale_weak(add_dimers(ice-twisted, 600), 62)",
                              variant(twisted_base, lambda m, n: f17("generate2").rescale_weak(f17("generate2").add_dimers(m, n, 600), 62))),
    "ice-twisted-w90-dimer": ("qpu-f17/generate2.py rescale_weak(add_dimers(ice-twisted, 600), 90)",
                              variant(twisted_base, lambda m, n: f17("generate2").rescale_weak(f17("generate2").add_dimers(m, n, 600), 90))),
    "ice-twisted-w62-dimer1200": ("qpu-f17/generate2.py and generate6.py rescale_weak(add_dimers(ice-twisted, 1200), 62)", wave(pairs=1200, weak=62)),
    "ice-twisted-w62-dimer1800": ("qpu-f17/generate6.py rescale_weak(add_dimers(ice-twisted, 1800), 62)", wave(pairs=1800, weak=62)),
    "ice-twisted-w62-dimer1350": ("qpu-f17/wave_run.py build --pairs 1350 --weak 62", wave(pairs=1350, weak=62)),
    "ice-twisted-w62-dimer1500": ("qpu-f17/wave_run.py build --pairs 1500 --weak 62", wave(pairs=1500, weak=62)),
    "ice-twisted-w45-dimer1200": ("qpu-f17/wave_run.py build --pairs 1200 --weak 45", wave(pairs=1200, weak=45)),
    "ice-twisted-w90-dimer1200": ("qpu-f17/wave_run.py build --pairs 1200 --weak 90", wave(pairs=1200, weak=90)),
    "ice-twisted-w62-dimer1200-p500": ("qpu-f17/wave_run.py build --pairs 1200 --weak 62 --pair-milli -500", wave(pairs=1200, weak=62, pair_milli=-500)),
    "ice-twisted-w62-loop4x300": ("qpu-f17/wave_run.py build --loops 300 --loop-length 4 --weak 62", wave(loops=300, loop_length=4, weak=62)),
    "ice-twisted-L9": ("qpu-f17/window.py cut(ice-twisted, 9)", twisted_window(9)),
    "ice-twisted-L13": ("qpu-f17/window.py cut(ice-twisted, 13)", twisted_window(13)),
    "ice-twisted-L18": ("qpu-f17/window.py cut(ice-twisted, 18)", twisted_window(18)),
    "ice-twisted-w62-dimer1200-L9": ("qpu-f17/generate7.py window 9 of ice-twisted-w62-dimer1200", dimer1200_window(9)),
    "ice-twisted-w62-dimer1200-L13": ("qpu-f17/generate7.py window 13 of ice-twisted-w62-dimer1200", dimer1200_window(13)),
    "ice-twisted-w62-dimer1200-L18": ("qpu-f17/generate7.py window 18 of ice-twisted-w62-dimer1200", dimer1200_window(18)),
    "ice-twisted-w62-dimer1200-L22": ("qpu-f17/generate7.py window 22 of ice-twisted-w62-dimer1200", dimer1200_window(22)),
    "ice-overlap2": ("qpu-f17/generate3.py main body, cell ice-overlap2", k4_cell("ice-overlap2")),
    "ice-all-k4": ("qpu-f17/generate3.py main body, cell ice-all-k4", k4_cell("ice-all-k4")),
    "ice-charge": ("qpu-f17/generate3.py main body, cell ice-charge", k4_cell("ice-charge")),
    "parity-k44": ("qpu-f17/generate5.py main body", parity_k44),
    "cbfm-h2": ("qpu-f17/generate.py generate('cbfm-h2', ...)", cbfm_f17("cbfm-h2")),
    "cbfm-z": ("qpu-f17/generate.py generate('cbfm-z', ...)", cbfm_f17("cbfm-z")),
    "cbfm-p-f17": ("qpu-f17/generate.py generate('cbfm-p', ...)", cbfm_f17("cbfm-p")),
}

# ---------------------------------------------------------------- new-pow family generators
# Each wrapper repeats the per-nonce body of the family's main() or generate_all(): generate,
# census, admission, and the attempt loop where the family has one. powlib families save with
# np.savez (uncompressed), famlib families with np.savez_compressed.


def family(name: str, file: str = "generate.py"):
    key = "fam_" + name.replace("-", "_")
    if key not in sys.modules:
        load_file_module(key, FAMILIES / name / file)
    return sys.modules[key]


def admitted_or_reject(adm: dict) -> None:
    if not adm["admitted"]:
        raise Rejected(json.dumps({k: v for k, v in adm.items() if v is not True}, default=str))


def powlib_build(name: str, kind: str, admit: bool = True) -> Callable[[str], dict]:
    """main() of dimers-*, four-parity, ldpc and xzzx: generate, census, admission."""

    def build(nonce: str) -> dict:
        m = family(name)
        if kind == "stats":
            arrays, _, stats = m.generate_model(nonce)
            adm = m.admission(m.census(arrays, stats))
        else:
            arrays, dm, meta = m.generate_model(nonce)
            adm = m.admission(m.census(arrays, dm, meta))
        m.powlib.validate_model(arrays)
        if admit:
            admitted_or_reject(adm)
        return arrays

    return build


def pairs_constraint(count: int) -> Callable[[str], dict]:
    def build(nonce: str) -> dict:
        m = family("ice-pairs-constraint")
        arrays, stats = m.generate_model(nonce, count)
        admitted_or_reject(m.admission(m.census(arrays, stats)))
        m.powlib.validate_model(arrays)
        return arrays

    return build


def fields_h0(magnitude: int) -> Callable[[str], dict]:
    """ice-fields-h500-h0/generate.py run(): the parent is the suite ice-fields model of the nonce."""

    def build(nonce: str) -> dict:
        m = family("ice-fields-h500-h0")
        if not getattr(m, "_parent_patched", False):
            original = m.load_parent

            def load_parent(n: str, mag: int) -> dict:
                if m.parent_path(n, mag).exists():
                    return original(n, mag)
                return suite("ice_fields").generate_model(n, mag)

            m.load_parent = load_parent
            m._parent_patched = True
        arrays, stats = m.generate_model(nonce, magnitude)
        c = m.census(arrays, magnitude)
        c["candidates_min"] = stats["candidates_min"]
        c["candidates_median"] = stats["candidates_median"]
        admitted_or_reject(m.admission(c, arrays))
        m.powlib.validate_model(arrays)
        return arrays

    return build


def ice_stack(layers: int) -> Callable[[str], dict]:
    def build(nonce: str) -> dict:
        m = family("ice-stack-2")
        model = m.generate_model(nonce, layers)
        adm = m.admission(model)
        if m.census(model)["vertical_fraction"] < m.MIN_VERTICAL_FRACTION:
            adm["admitted"] = False
            adm["reason"] = "vertical fraction below 0.40"
        admitted_or_reject(adm)
        m.powlib.validate_model(model)
        return model

    return build


def dimer_template(which: str) -> Callable[[str], dict]:
    """dimer-template-half-fill/generate.py build(): admission needs the strong-layer MILP."""

    def build(nonce: str) -> dict:
        m = family("dimer-template-half-fill")
        tpl = m.TEMPLATES[which]
        arrays, stats = m.generate_model(nonce, tpl)
        c = m.census(arrays, stats)
        checks = m.structure_checks(arrays)
        strong = m.exact_milp(arrays, 60.0, mode="strong")
        admitted_or_reject(m.admission(c, checks, strong))
        m.powlib.validate_model(arrays)
        return arrays

    return build


def attempts_family(name: str, errors: tuple) -> Callable[[str], dict]:
    """generate_all() of cover-template-half-fill and paint-shop-euler: up to MAX_ATTEMPTS."""

    def build(nonce: str) -> dict:
        m = family(name)
        reasons = []
        for attempt in range(m.MAX_ATTEMPTS):
            try:
                arrays, stats = m.generate_model(nonce, attempt)
            except errors as exc:
                reasons.append(f"attempt {attempt}: {exc}")
                continue
            if not stats["admission"]["admitted"]:
                reasons.append(f"attempt {attempt}: {stats['admission']}")
                continue
            m.powlib.validate_model(arrays)
            return arrays
        raise Rejected("; ".join(reasons))

    return build


def famlib_family(name: str) -> Callable[[str], dict]:
    """famlib.generate_all() inner loop and famlib.save_model() arrays, max_attempts 4."""

    def build(nonce: str) -> dict:
        m = family(name)
        F = m.F
        reasons = []
        for attempt in range(4):
            result = m.build(nonce, attempt)
            if "reject_reason" in result.info:
                reasons.append(f"attempt {attempt}: {result.info['reject_reason']}")
                continue
            try:
                model = F.build_model(result.energy, result.extra)
            except F.Overflow as exc:
                reasons.append(f"attempt {attempt}: overflow: {exc}")
                continue
            census = F.model_census(model, result.members, result.aux)
            census.update(result.info)
            adm = F.admission(census, result.max_chain)
            if not adm["admitted"]:
                reasons.append(f"attempt {attempt}: admission {adm}")
                continue
            topo = F.topology()
            arrays = {
                "edges": model.edges,
                "j_milli": model.j_milli,
                "h_milli": model.h_milli,
                "physical_nodes": topo.physical,
                "energy_offset_quarter_milli": np.asarray(model.const4, dtype=np.int64),
                "family": np.asarray(name),
                "nonce": np.asarray(nonce),
                "topology_sha256": np.asarray(topo.digest.hex()),
            }
            for key, value in model.extra.items():
                arrays[key] = np.asarray(value)
            return arrays
        raise Rejected("; ".join(reasons))

    return build


FAMILY_CELLS: dict[str, tuple[str, Callable[[str], dict]]] = {
    "dimers-fullchip": ("new-pow/families/dimers-fullchip/generate.py generate_model, census, admission", powlib_build("dimers-fullchip", "stats")),
    "dimers-interacting": ("new-pow/families/dimers-interacting/generate.py generate_model, census, admission", powlib_build("dimers-interacting", "stats")),
    "four-parity-interacting": ("new-pow/families/four-parity-interacting/generate.py generate_model, census, admission", powlib_build("four-parity-interacting", "dm")),
    "ldpc-small-row": ("new-pow/families/ldpc-small-row/generate.py generate_model, census, admission", powlib_build("ldpc-small-row", "dm")),
    "xzzx-decoding": ("new-pow/families/xzzx-decoding/generate.py generate_model (admission recorded, not applied: the 12 saved models all fail it)",
                      powlib_build("xzzx-decoding", "dm", admit=False)),
    "ice-pairs-constraint-1350": ("new-pow/families/ice-pairs-constraint/generate.py generate_model(nonce, 1350), census, admission", pairs_constraint(1350)),
    "ice-fields-h500-h0": ("new-pow/families/ice-fields-h500-h0/generate.py generate_model(nonce, 500), parent from qpu-suite/work/ice_fields.py", fields_h0(500)),
    "ice-fields-h1000-h0": ("new-pow/families/ice-fields-h500-h0/generate.py generate_model(nonce, 1000), parent from qpu-suite/work/ice_fields.py", fields_h0(1000)),
    "ice-stack-2": ("new-pow/families/ice-stack-2/generate.py generate_model(nonce, 2), admission, vertical fraction", ice_stack(2)),
    "ice-stack-3": ("new-pow/families/ice-stack-2/generate.py generate_model(nonce, 3), admission, vertical fraction", ice_stack(3)),
    "dimer-template-half-fill": ("new-pow/families/dimer-template-half-fill/generate.py template 'interacting'", dimer_template("interacting")),
    "dimer-template-half-fill-fullchip": ("new-pow/families/dimer-template-half-fill/generate.py template 'fullchip'", dimer_template("fullchip")),
    "cover-template-half-fill": ("new-pow/families/cover-template-half-fill/generate.py generate_all attempt loop", attempts_family("cover-template-half-fill", (OverflowError,))),
    "paint-shop-euler": ("new-pow/families/paint-shop-euler/generate.py generate_all attempt loop", attempts_family("paint-shop-euler", (OverflowError, ValueError, RuntimeError))),
    "exact-cover-mixed": ("new-pow/families/exact-cover-mixed/generate.py build via famlib.generate_all", famlib_family("exact-cover-mixed")),
    "nae3-signed": ("new-pow/families/nae3-signed/generate.py build via famlib.generate_all", famlib_family("nae3-signed")),
    "or-sat-native": ("new-pow/families/or-sat-native/generate.py build via famlib.generate_all", famlib_family("or-sat-native")),
    "tait-coloring": ("new-pow/families/tait-coloring/generate.py build via famlib.generate_all", famlib_family("tait-coloring")),
    "two-commodity-routing": ("new-pow/families/two-commodity-routing/generate.py build via famlib.generate_all", famlib_family("two-commodity-routing")),
    "wmis-native": ("new-pow/families/wmis-native/generate.py build via famlib.generate_all", famlib_family("wmis-native")),
}
SAVERS = {cell: np.savez for cell, (desc, _) in FAMILY_CELLS.items() if "famlib" not in desc}


def k88_generate(nonce: str) -> dict:
    """The dcl-k88 generator, loaded by path because its directory name is not an importable module name."""
    module = load_file_module("dcl_k88_generate", NP / "dcl-k88" / "generate.py")
    return module.generate_model(nonce)


K88_CELLS = {"dcl-k88": ("new-pow/dcl-k88/generate.py generate_model(nonce), k88-live-v2", k88_generate)}

GENERATORS: dict[str, tuple[str, Callable[[str], dict]]] = {**SUITE_CELLS, **F17_CELLS, **FAMILY_CELLS, **K88_CELLS}

# ---------------------------------------------------------------- driver


def targets(cell: str) -> list[dict]:
    import rescore  # pyright: ignore[reportMissingImports]

    path, keys = rescore.CELLS[cell]
    data = json.loads(Path(path).read_text())
    return [row for key in keys for row in data[key]]


def compare(model: dict[str, np.ndarray], saved: dict[str, np.ndarray]) -> list[str]:
    diffs = []
    if set(model) != set(saved):
        diffs.append(f"keys differ: generated only {sorted(set(model) - set(saved))}, saved only {sorted(set(saved) - set(model))}")
    for key in sorted(set(model) & set(saved)):
        a, b = np.asarray(model[key]), saved[key]
        if a.shape != b.shape or not np.array_equal(a, b):
            diffs.append(f"{key} differs (shape {a.shape} vs {b.shape})")
        elif a.dtype != b.dtype:
            diffs.append(f"{key} dtype {a.dtype} vs {b.dtype}")
    return diffs


def check(cell: str, limit: int | None = None) -> dict:
    _, generate = GENERATORS[cell]
    rows = targets(cell)[:limit]
    results = []
    t0 = time.time()
    for row in rows:
        saved = load_npz(Path(row["model"]))
        same_bytes = False
        try:
            model = generate(row["nonce"])
            diffs = compare(model, saved)
            buffer = io.BytesIO()
            SAVERS.get(cell, np.savez_compressed)(buffer, **model)
            same_bytes = buffer.getvalue() == Path(row["model"]).read_bytes()
        except Rejected as exc:
            diffs = [f"generator rejected a saved nonce: {exc}"]
        results.append({"nonce": row["nonce"], "diffs": diffs, "bytes_identical": same_bytes})
        print(cell, "check", row["nonce"][:12], "ok" if not diffs else diffs, "bytes", same_bytes, flush=True)
    bad = [r for r in results if r["diffs"]]
    return {"checked": len(results), "reproduced": len(results) - len(bad), "mismatches": bad,
            "bytes_identical": sum(r["bytes_identical"] for r in results),
            "passed": not bad, "seconds": round(time.time() - t0, 1)}


@lru_cache(maxsize=1)
def topology():
    sys.path.insert(0, str(F17))
    worker = load_file_module("capture_worker_f17", F17 / "capture_worker_f17.py")
    return worker, worker.Topology(str(SPEC))


def validate(cell: str, nonces: list[str]) -> None:
    """Run the worker's request parser and model loader over the new models."""
    worker, topo = topology()
    stage = HERE / ".validate" / cell
    models = stage / "work/cells" / cell / "models"
    models.mkdir(parents=True, exist_ok=True)
    for nonce in nonces:
        link = models / f"{nonce}.npz"
        if not link.exists():
            link.symlink_to(HERE / cell / "models" / f"{nonce}.npz")
    request = HERE / "requests" / f"{cell}-80.json"
    _, pairs = worker.parse_request(stage, request)
    for _, path in pairs:
        topo.load_model(path)


def census(model: dict[str, np.ndarray]) -> dict:
    edges = np.asarray(model["edges"])
    h = np.asarray(model.get("h_milli", np.zeros(0)))
    active = set(np.unique(edges).tolist()) | set(np.flatnonzero(h).tolist())
    return {"qubits": len(active), "couplers": int(len(edges))}


def extend(cell: str) -> dict:
    description, generate = GENERATORS[cell]
    rows = targets(cell)
    existing = [r["nonce"] for r in rows]
    record: dict = {"cell": cell, "generator": description, "existing": len(existing), "started": stamp()}
    record["check"] = check(cell)
    if not record["check"]["passed"]:
        record["status"] = "failed reproduction"
        return record
    out = HERE / cell / "models"
    out.mkdir(parents=True, exist_ok=True)
    need = TARGET - len(existing)
    new, index_map, rejected, stats = [], [], [], []
    i = 13
    while len(new) < need:
        nonce = nonce_for(i)
        if nonce in existing:
            raise ValueError("nonce collision")
        path = out / f"{nonce}.npz"
        try:
            if path.exists():
                model = load_npz(path)
            else:
                model = generate(nonce)
                SAVERS.get(cell, np.savez_compressed)(path, **model)
        except Rejected as exc:
            rejected.append({"index": i, "nonce": nonce, "reason": str(exc)})
            print(cell, "reject", i, exc, flush=True)
            i += 1
            continue
        new.append(nonce)
        index_map.append({"position": len(existing) + len(new), "index": i, "nonce": nonce})
        stats.append(census(model))
        print(cell, "model", len(existing) + len(new), "index", i, stats[-1], flush=True)
        i += 1
    requests = HERE / "requests"
    requests.mkdir(exist_ok=True)
    (requests / f"{cell}-80.json").write_text(json.dumps(
        {"cell": cell, "models_dir": f"work/cells/{cell}/models", "nonces": new, "reads": 48, "anneal_us": 80}) + "\n")
    validate(cell, new)
    saved_census = [census(load_npz(Path(r["model"]))) for r in rows]
    record.update({
        "status": "done", "new_models": len(new), "rejected": rejected, "index_map": index_map,
        "census_median_new": {k: float(np.median([s[k] for s in stats])) for k in ("qubits", "couplers")},
        "census_median_existing": {k: float(np.median([s[k] for s in saved_census])) for k in ("qubits", "couplers")},
        "worker_validation": "passed", "finished": stamp(),
    })
    return record


def main(argv: list[str]) -> int:
    mode, cells = argv[0], argv[1:]
    failures = 0
    for cell in cells:
        try:
            if mode in ("check", "probe"):
                result = {"cell": cell, "check": check(cell, 1 if mode == "probe" else None)}
            else:
                result = extend(cell)
        except Exception as exc:  # recorded per cell so one failure does not stop the group
            result = {"cell": cell, "status": "error", "error": f"{type(exc).__name__}: {exc}",
                      "traceback": traceback.format_exc()}
        name = {"check": "check.json", "probe": "probe.json"}.get(mode, "record.json")
        (HERE / cell).mkdir(exist_ok=True)
        (HERE / cell / name).write_text(json.dumps(result, indent=1) + "\n")
        status = result.get("status") or ("passed" if result["check"]["passed"] else "failed reproduction")
        failures += status != "done" and status != "passed"
        with (HERE / ("progress.md" if mode == "extend" else ".checks.log")).open("a") as fh:
            fh.write(f"- {stamp()} {cell}: {mode} {status}"
                     + (f", {result.get('new_models')} new models" if status == "done" else "") + "\n")
        print("RESULT", cell, status, flush=True)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
