#!/usr/bin/env python3
"""Independent rescore of a classical solver's best states against the production QPU targets.

Usage: rescore.py CELL MEASUREMENT_DIR

CELL is one of the campaign cells (see README.md). MEASUREMENT_DIR holds one ``<nonce>.best.npy``
per model (int8, +1/-1, 4,575 entries in the chip frame, index = physical_nodes order of the
model file) and a ``results.jsonl`` with one row per model: ``{"nonce", "seconds", "threads",
...}``. ``seconds`` is the wall time of the whole per-model run that produced the state.

Prints one row per model and the tier verdict. Writes ``MEASUREMENT_DIR/rescore.json``.
Missing states count as losses. Energies are integer milli from the saved model, never from the
solver's own report.
"""

from __future__ import annotations

import json
import statistics
import sys
from pathlib import Path

import numpy as np

F17_TARGETS = Path("/home/carback1/quip-data/regimes/qpu-f17/adversary/targets.json")
SUITE_TARGETS = Path(__file__).resolve().parent / "targets-suite.json"
F17_EXTRA_TARGETS = Path(__file__).resolve().parent / "targets-f17-extra.json"
UNCAPTURED_TARGETS = Path(__file__).resolve().parent / "targets-uncaptured.json"
FAMILY_TARGETS = Path(__file__).resolve().parent / "targets-families.json"
ROUND2_TARGETS = Path(__file__).resolve().parent / "targets-round2.json"
SUITE7_TARGETS = Path(__file__).resolve().parent / "targets-suite7.json"
ROUND3_TARGETS = Path(__file__).resolve().parent / "targets-round3.json"
H0_TARGETS = Path(__file__).resolve().parent / "targets-h0.json"
K88_TARGETS = Path(__file__).resolve().parent / "targets-k88.json"
# cell -> (targets file, keys inside it)
CELLS = {
    "ice-twisted-w62-dimer1200": (F17_TARGETS, ["ice-twisted-w62-dimer1200", "ice-twisted-w62-dimer1200-ext"]),
    "ice-twisted-w62-dimer": (F17_TARGETS, ["ice-twisted-w62-dimer"]),
    "ice-twisted-w62-dimer1350": (F17_TARGETS, ["ice-twisted-w62-dimer1350", "ice-twisted-w62-dimer1350-ext"]),
    "ice-twisted-w62-dimer1800": (F17_TARGETS, ["ice-twisted-w62-dimer1800", "ice-twisted-w62-dimer1800-ext"]),
    "ice-twisted": (F17_TARGETS, ["ice-twisted"]),
    "ice-fields-h500": (SUITE_TARGETS, ["ice-fields-h500"]),
    "ice-fields-h1000": (SUITE_TARGETS, ["ice-fields-h1000"]),
    "ice-glass-r8": (SUITE_TARGETS, ["ice-glass-r8"]),
    "ice-defect-p03125": (SUITE_TARGETS, ["ice-defect-p03125"]),
    "ice-defect-p015625": (SUITE_TARGETS, ["ice-defect-p015625"]),
    "ice-twisted-w62": (F17_TARGETS, ["ice-twisted-w62"]),
    "ice-twisted-dimer": (F17_TARGETS, ["ice-twisted-dimer"]),
    "ice-twisted-w90-dimer": (F17_TARGETS, ["ice-twisted-w90-dimer"]),
    "ice-twisted-w62-dimer1200-L9": (F17_TARGETS, ["ice-twisted-w62-dimer1200-L9"]),
    "ice-twisted-w62-dimer1200-L13": (F17_TARGETS, ["ice-twisted-w62-dimer1200-L13"]),
    "ice-twisted-w62-dimer1200-L18": (F17_TARGETS, ["ice-twisted-w62-dimer1200-L18"]),
    "ice-twisted-w62-dimer1200-L22": (F17_TARGETS, ["ice-twisted-w62-dimer1200-L22"]),
    "ice-twisted-w62-dimer1500": (F17_TARGETS, ["ice-twisted-w62-dimer1500"]),
    "ice-twisted-w45-dimer1200": (F17_TARGETS, ["ice-twisted-w45-dimer1200"]),
    "ice-twisted-w90-dimer1200": (F17_TARGETS, ["ice-twisted-w90-dimer1200"]),
    "ice-twisted-w62-loop4x300": (F17_TARGETS, ["ice-twisted-w62-loop4x300"]),
    "ice-twisted-w62-dimer1200-p500": (F17_TARGETS, ["ice-twisted-w62-dimer1200-p500"]),
    "ice-defect-p0625": (SUITE_TARGETS, ["ice-defect-p0625"]),
    "dimer-r7": (SUITE_TARGETS, ["dimer-r7"]),
    "cbfm-p": (SUITE_TARGETS, ["cbfm-p"]),
    "cubic-no-dimer": (SUITE_TARGETS, ["cubic-no-dimer"]),
    "diamond-3d": (SUITE_TARGETS, ["diamond-3d"]),
    "dimer-power-mixed": (SUITE_TARGETS, ["dimer-power-mixed"]),
    "dimer-r5": (SUITE_TARGETS, ["dimer-r5"]),
    "dimer-r6": (SUITE_TARGETS, ["dimer-r6"]),
    "ice-defect-p125": (SUITE_TARGETS, ["ice-defect-p125"]),
    "ice-defect-p25": (SUITE_TARGETS, ["ice-defect-p25"]),
    "ice-defect-p50": (SUITE_TARGETS, ["ice-defect-p50"]),
    "ice-glass-r5": (SUITE_TARGETS, ["ice-glass-r5"]),
    "ice-overlap-c2": (SUITE_TARGETS, ["ice-overlap-c2"]),
    "ice-overlap-c3": (SUITE_TARGETS, ["ice-overlap-c3"]),
    "ice-three-level": (SUITE_TARGETS, ["ice-three-level"]),
    "kagome-net": (SUITE_TARGETS, ["kagome-net"]),
    "strong-net-d1.5": (SUITE_TARGETS, ["strong-net-d1.5"]),
    "strong-net-d2": (SUITE_TARGETS, ["strong-net-d2"]),
    "strong-net-d3": (SUITE_TARGETS, ["strong-net-d3"]),
    "tri-match-power": (SUITE_TARGETS, ["tri-match-power"]),
    "tri-match-r4": (SUITE_TARGETS, ["tri-match-r4"]),
    "tri-match-r6": (SUITE_TARGETS, ["tri-match-r6"]),
    "cbfm-h2": (F17_EXTRA_TARGETS, ["cbfm-h2"]),
    "cbfm-z": (F17_EXTRA_TARGETS, ["cbfm-z"]),
    "ice-all-k4": (F17_EXTRA_TARGETS, ["ice-all-k4"]),
    "ice-charge": (F17_EXTRA_TARGETS, ["ice-charge"]),
    "ice-glass-r16": (F17_EXTRA_TARGETS, ["ice-glass-r16"]),
    "ice-overlap2": (F17_EXTRA_TARGETS, ["ice-overlap2"]),
    "ice-random-k4": (F17_EXTRA_TARGETS, ["ice-random-k4"]),
    "ice-twisted-L13": (F17_EXTRA_TARGETS, ["ice-twisted-L13"]),
    "ice-twisted-L18": (F17_EXTRA_TARGETS, ["ice-twisted-L18"]),
    "ice-twisted-L9": (F17_EXTRA_TARGETS, ["ice-twisted-L9"]),
    "ice-twisted-dimer1200": (F17_EXTRA_TARGETS, ["ice-twisted-dimer1200"]),
    "ice-twisted-w250": (F17_EXTRA_TARGETS, ["ice-twisted-w250"]),
    "ice-twisted-w31": (F17_EXTRA_TARGETS, ["ice-twisted-w31"]),
    "ice-twisted-w45": (F17_EXTRA_TARGETS, ["ice-twisted-w45"]),
    "ice-twisted-w90": (F17_EXTRA_TARGETS, ["ice-twisted-w90"]),
    "parity-k44": (F17_EXTRA_TARGETS, ["parity-k44"]),
    "cbfm-p-f17": (F17_EXTRA_TARGETS, ["cbfm-p-f17"]),
    "pair-loop": (UNCAPTURED_TARGETS, ["pair-loop"]),
    "strip-glass": (UNCAPTURED_TARGETS, ["strip-glass"]),
    "frust-dense-k12": (UNCAPTURED_TARGETS, ["frust-dense-k12"]),
    "frust-dense-k16": (UNCAPTURED_TARGETS, ["frust-dense-k16"]),
    "exact-cover-mixed": (FAMILY_TARGETS, ["exact-cover-mixed"]),
    "tait-coloring": (FAMILY_TARGETS, ["tait-coloring"]),
    "or-sat-native": (FAMILY_TARGETS, ["or-sat-native"]),
    "nae3-signed": (FAMILY_TARGETS, ["nae3-signed"]),
    "two-commodity-routing": (FAMILY_TARGETS, ["two-commodity-routing"]),
    "xzzx-decoding": (FAMILY_TARGETS, ["xzzx-decoding"]),
    "ldpc-small-row": (FAMILY_TARGETS, ["ldpc-small-row"]),
    "four-parity-interacting": (FAMILY_TARGETS, ["four-parity-interacting"]),
    "dimers-interacting": (FAMILY_TARGETS, ["dimers-interacting"]),
    "ice-pairs-constraint-1350": (ROUND2_TARGETS, ["ice-pairs-constraint-1350"]),
    "dimers-fullchip": (ROUND2_TARGETS, ["dimers-fullchip"]),
    "dcl-k44-l5": (SUITE7_TARGETS, ["dcl-k44-l5"]),
    "dcl-k44-l7": (SUITE7_TARGETS, ["dcl-k44-l7"]),
    "fcl-planted-a0.2": (SUITE7_TARGETS, ["fcl-planted-a0.2"]),
    "fcl-planted-a0.3": (SUITE7_TARGETS, ["fcl-planted-a0.3"]),
    "ice-planted": (SUITE7_TARGETS, ["ice-planted"]),
    "wsc": (SUITE7_TARGETS, ["wsc"]),
    "ice-stack-2": (ROUND3_TARGETS, ["ice-stack-2"]),
    "ice-stack-3": (ROUND3_TARGETS, ["ice-stack-3"]),
    "wmis-native": (ROUND3_TARGETS, ["wmis-native"]),
    "ice-fields-h500-h0": (H0_TARGETS, ["ice-fields-h500-h0"]),
    "ice-fields-h1000-h0": (H0_TARGETS, ["ice-fields-h1000-h0"]),
    "dimer-template-half-fill": (H0_TARGETS, ["dimer-template-half-fill"]),
    "dimer-template-half-fill-fullchip": (H0_TARGETS, ["dimer-template-half-fill-fullchip"]),
    "cover-template-half-fill": (H0_TARGETS, ["cover-template-half-fill"]),
    "paint-shop-euler": (H0_TARGETS, ["paint-shop-euler"]),
    "dcl-k88": (K88_TARGETS, ["dcl-k88"]),
}
QPU_WALL_S = 0.1
TIER2_WALL_S = 1200.0


def energy(spins: np.ndarray, model: dict[str, np.ndarray]) -> int:
    s = spins.astype(np.int64)
    if s.shape != model["h_milli"].shape or not np.all(np.abs(s) == 1):
        raise ValueError(f"state must be {model['h_milli'].shape} of +1/-1, got {s.shape}")
    edges = model["edges"]
    return int((s[edges[:, 0]] * s[edges[:, 1]]) @ model["j_milli"] + s @ model["h_milli"])


def main(cell: str, mdir: Path) -> int:
    targets_path, keys = CELLS[cell]
    targets = json.loads(targets_path.read_text())
    rows = [row for key in keys for row in targets[key]]
    timing: dict[str, dict] = {}
    results = mdir / "results.jsonl"
    if results.exists():
        for line in results.read_text().splitlines():
            if line.strip():
                rec = json.loads(line)
                timing[rec["nonce"]] = rec
    out = []
    for row in rows:
        nonce = row["nonce"]
        qpu = int(row["qpu80_best"])
        state = mdir / f"{nonce}.best.npy"
        rec = timing.get(nonce, {})
        if state.exists():
            with np.load(row["model"]) as data:
                model = {k: data[k] for k in ("edges", "j_milli", "h_milli")}
            e = energy(np.load(state), model)
            verdict = "win" if e < qpu else "tie" if e == qpu else "loss"
        else:
            e, verdict = None, "missing"
        out.append({
            "nonce": nonce, "qpu80_best": qpu, "classical": e, "verdict": verdict,
            "seconds": rec.get("seconds"), "threads": rec.get("threads"),
        })
        delta = "" if e is None else f"{e - qpu:+d}"
        secs = "" if rec.get("seconds") is None else f"{rec['seconds']:.3f}"
        print(f"{nonce[:12]}  qpu {qpu:>10}  cls {str(e):>10}  {delta:>8}  {verdict:<7}  {secs:>10} s  thr {rec.get('threads', '')}")
    n = len(rows)
    wins = sum(r["verdict"] == "win" for r in out)
    ties = sum(r["verdict"] == "tie" for r in out)
    secs = [r["seconds"] for r in out if r["seconds"] is not None]
    median_s = statistics.median(secs) if secs else None
    majority = n // 2 + 1
    tier = "hold"
    if median_s is not None and len(secs) == n:
        if wins >= majority and median_s <= QPU_WALL_S:
            tier = "tier1"
        elif wins + ties >= majority and median_s <= TIER2_WALL_S:
            tier = "tier2"
    deltas = [r["classical"] - r["qpu80_best"] for r in out if r["classical"] is not None]
    margin = None
    if deltas:
        margin = {"median": statistics.median(deltas), "deepest": min(deltas), "shallowest": max(deltas)}
    summary = {
        "cell": cell, "models": n, "wins": wins, "ties": ties,
        "losses": n - wins - ties, "median_seconds": median_s,
        "ratio_to_qpu_access": None if median_s is None else median_s / 0.046,
        "tier": tier, "margin_milli": margin, "rows": out,
    }
    (mdir / "rescore.json").write_text(json.dumps(summary, indent=1))
    print(f"\n{cell}: {wins} wins, {ties} ties, {n - wins - ties} losses of {n}; "
          f"median {median_s} s wall; tier {tier}")
    if margin:
        print(f"margin milli (classical minus qpu80_best): median {margin['median']:+.0f}, "
              f"deepest {margin['deepest']:+d}, shallowest {margin['shallowest']:+d}")
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 3 or sys.argv[1] not in CELLS:
        print(__doc__)
        print("cells:", ", ".join(CELLS))
        sys.exit(2)
    sys.exit(main(sys.argv[1], Path(sys.argv[2])))
