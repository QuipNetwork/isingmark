#!/usr/bin/env python3
"""Write new-pow/targets-pp-100.json: the postprocessed QPU reference on the 100 models of every captured cell.

Usage: write_targets_pp_100.py run [--workers N] [--cells a,b]
       write_targets_pp_100.py cell FAMILY CELL     (one cell, called by `run`)
       write_targets_pp_100.py assemble

Each cell runs its family optimizer at the family, method and budget of targets-pp.json on all 100 models of
targets-greedy-100.json, one thread per process. The optimizers read their models from targets-greedy.json (or
`descent.targets`); `cell` imports the optimizer's own run.py and points that lookup at the 100-model rows, so
the optimizer code runs unchanged. Results and every state go to
measurements/optimizers-100/<family>/<cell>/ (results.jsonl, states/). A cell without a budget in targets-pp.json
keeps the greedy reference (`qpu80_pp` = `qpu80_greedy`).

`assemble` writes the 100 rows per cell. The 12 rows of targets-pp.json are copied unchanged; the other rows take
`qpu80_pp` = min(`pp_best`, `qpu80_greedy`) and `pp_cpu_s` = `cpu_s` of the rerun. The rerun of the 12 copied
models is the control. Energies are in milli. Run with the project venv.
"""
from __future__ import annotations

import argparse
import importlib
import json
import subprocess
import sys
import time
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from statistics import mean, median

HERE = Path(__file__).resolve().parent
NP = HERE.parent
OPT = HERE / "optimizers"
MEAS = HERE / "measurements" / "optimizers-100"
GREEDY_100 = NP / "targets-greedy-100.json"
PP = NP / "targets-pp.json"
TARGET = NP / "targets-pp-100.json"
PY = "/home/carback1/quip-miner-dwave-cascade/.venv/bin/python"


def plan() -> dict[str, dict]:
    return json.loads(PP.read_text())


def out_dir(family: str, cell: str) -> Path:
    return MEAS / family / cell


def argv(family: str, cell: str, method: str, budget: float) -> list[str]:
    out = str(out_dir(family, cell))
    common = ["run.py", "--cell", cell, "--budget-s", f"{budget:g}", "--out", out, "--models", "all",
              "--threads", "1"]
    extra = {"constraints": [], "dimers": ["--method", method], "glasses": ["--methods", method],
             "ice-pairs": ["--method", method], "ice-tiles": ["--spec", method]}[family]
    return common + extra


def run_cell(family: str, cell: str) -> int:
    """Run one cell in this process with the optimizer's run.py, its model lookup patched to the 100 rows."""
    p = plan()[cell]
    rows = json.loads(GREEDY_100.read_text())[cell]
    sys.path[0] = str(OPT / family)
    run = importlib.import_module("run")
    base_archive = run.D.archive

    def archive(cell_: str, nonce: str) -> Path | None:
        # `BASE_ARCHIVES` pins cbfm-p and cbfm-p-f17 to their 12-model archives; the new reads are in qpu-recap
        arch = base_archive(cell_, nonce)
        f = NP / "qpu-recap" / "qpu" / cell_ / "qpu-80" / f"{nonce}.npz"
        return arch if arch is not None else (f if f.exists() else None)

    setattr(run.D, "archive", archive)
    if family == "constraints":
        setattr(run, "model_rows", lambda _cell, _which: [dict(r) for r in rows])
    elif family == "ice-tiles":
        setattr(run, "GREEDY", GREEDY_100)
        setattr(run.D, "targets", lambda _cell: [dict(r) for r in rows])
    else:
        setattr(run, "TARGETS", GREEDY_100)
    sys.argv = argv(family, cell, p["method"], p["budget_s"])
    return int(run.main() or 0)


def results(family: str, cell: str) -> dict[str, dict]:
    res = out_dir(family, cell) / "results.jsonl"
    if not res.exists():
        return {}
    return {r["nonce"]: r for r in map(json.loads, res.read_text().splitlines()) if r}


def progress(line: str) -> None:
    with (MEAS / "progress.md").open("a") as f:
        f.write(line + "\n")


def one_cell(cell: str, p: dict) -> str:
    family = p["family"]
    out_dir(family, cell).mkdir(parents=True, exist_ok=True)
    log = out_dir(family, cell) / "run.log"
    t0 = time.time()
    with log.open("a") as f:
        rc = subprocess.run([PY, str(Path(__file__).resolve()), "cell", family, cell], cwd=OPT / family,
                            stdout=f, stderr=subprocess.STDOUT, check=False).returncode
    n = len(results(family, cell))
    stamp = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    line = (f"- {stamp} `{cell}` ({family}, {p['method']}, {p['budget_s']} s): {n} of 100 models, "
            f"exit {rc}, wall {time.time() - t0:.0f} s")
    progress(line)
    return line


def run(workers: int, only: list[str] | None) -> None:
    MEAS.mkdir(parents=True, exist_ok=True)
    pp = plan()
    todo = [c for c in (only or sorted(pp)) if pp[c]["budget_s"] is not None and len(results(pp[c]["family"], c)) < 100]
    # longest budgets first, so the tail of the run is short
    todo.sort(key=lambda c: -pp[c]["budget_s"])
    print(len(todo), "cells to run", flush=True)
    with ThreadPoolExecutor(workers) as ex:
        for line in ex.map(lambda c: one_cell(c, pp[c]), todo):
            print(line, flush=True)


def assemble() -> None:
    pp = plan()
    greedy = json.loads(GREEDY_100.read_text())
    out: dict[str, dict] = {}
    control: dict[str, dict] = {}
    for cell in sorted(pp):
        p = pp[cell]
        old = {r["nonce"]: r for r in p["rows"]}
        if p["budget_s"] is None:
            rows = [old.get(g["nonce"]) or {"nonce": g["nonce"], "model": g["model"], "qpu80_best": g["qpu80_best"],
                                            "qpu80_greedy": g["qpu80_greedy"], "qpu80_pp": g["qpu80_greedy"],
                                            "pp_cpu_s": None} for g in greedy[cell]]
            out[cell] = {"family": p["family"], "method": p["method"], "budget_s": None, "rows": rows}
            continue
        res = results(p["family"], cell)
        rows = []
        diffs = []
        for g in greedy[cell]:
            n = g["nonce"]
            r = res.get(n)
            if r is None:
                raise RuntimeError(f"{cell} {n[:8]}: no optimizer row")
            pp_e = min(int(r["pp_best"]), int(g["qpu80_greedy"]))
            if n in old:
                rows.append(old[n])
                diffs.append((old[n]["qpu80_pp"], pp_e))
            else:
                rows.append({"nonce": n, "model": g["model"], "qpu80_best": g["qpu80_best"],
                             "qpu80_greedy": g["qpu80_greedy"], "qpu80_pp": pp_e, "pp_cpu_s": r["cpu_s"]})
        method = Counter(r["method"] for r in res.values()).most_common(1)[0][0]
        out[cell] = {"family": p["family"], "method": method, "budget_s": p["budget_s"], "rows": rows}
        control[cell] = {"models": len(diffs), "mean_abs_diff": mean(abs(a - b) for a, b in diffs),
                         "median_stored": median(a for a, _ in diffs), "median_rerun": median(b for _, b in diffs)}
    TARGET.write_text(json.dumps(out, indent=1) + "\n")
    (MEAS / "control.json").write_text(json.dumps(control, indent=1) + "\n")
    print(len(out), "cells,", sum(len(v["rows"]) for v in out.values()), "models ->", TARGET)


def main() -> int:
    if len(sys.argv) == 4 and sys.argv[1] == "cell":
        return run_cell(sys.argv[2], sys.argv[3])
    ap = argparse.ArgumentParser()
    ap.add_argument("step", choices=["run", "assemble"])
    ap.add_argument("--workers", type=int, default=5)
    ap.add_argument("--cells", default=None)
    a = ap.parse_args()
    if a.step == "run":
        run(min(a.workers, 5), a.cells.split(",") if a.cells else None)
    else:
        assemble()
    return 0


if __name__ == "__main__":
    sys.exit(main())
