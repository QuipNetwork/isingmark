"""Index the archived research rows of the seven selected cells.

Reads the research target manifest ``new-pow/targets-pp-100.json`` under the research root and
writes ``evaluation/qblock/archive-index.json``: for each cell, its rows in archive order with
the model path relative to the research root, the blake3 digest of the model file, and the
archived energies. Energies are decimal strings. The research root is read only.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

from blake3 import blake3

SCHEMA_REVISION = 1
DEFAULT_ROOT = Path("/home/carback1/quip-data/regimes")
SOURCE = "new-pow/targets-pp-100.json"
DEFAULT_OUT = Path(__file__).resolve().parents[3] / "evaluation" / "qblock" / "archive-index.json"
# The seven selected cells, in index order.
CELLS = (
    "ice-charge",
    "ice-fields-h500",
    "ice-fields-h1000",
    "two-commodity-routing",
    "ice-twisted-w62-dimer1200",
    "ice-twisted-w62-dimer1350",
    "ice-twisted-w45-dimer1200",
)
ENERGY_FIELDS = ("qpu80_best", "qpu80_greedy", "qpu80_pp")


def blake3_hex(path: Path) -> str:
    return blake3(path.read_bytes()).hexdigest()


def relative_model(model: str, root: Path) -> str:
    """``model`` relative to ``root``. A path outside the root is an error."""
    try:
        return Path(model).relative_to(root).as_posix()
    except ValueError:
        raise ValueError(f"model {model} is not under {root}") from None


def build_index(root: Path, cache: dict[str, str] | None = None) -> dict[str, Any]:
    """The archive index for ``root``. ``cache`` maps a model path to its digest."""
    cache = {} if cache is None else cache
    manifest = root / SOURCE
    targets = json.loads(manifest.read_text(encoding="utf-8"))
    cells = []
    for cell in CELLS:
        if cell not in targets:
            raise ValueError(f"cell {cell} is not in {SOURCE}")
        target = targets[cell]
        rows = []
        for row in target["rows"]:
            model = relative_model(row["model"], root)
            if model not in cache:
                cache[model] = blake3_hex(root / model)
            rows.append(
                {
                    "nonce": row["nonce"],
                    "model": model,
                    "model_blake3": cache[model],
                    **{field: str(int(row[field])) for field in ENERGY_FIELDS},
                    "pp_cpu_s": row["pp_cpu_s"],
                }
            )
        cells.append(
            {
                "cell": cell,
                "family": target["family"],
                "method": target["method"],
                "budget_s": target["budget_s"],
                "rows": rows,
            }
        )
    return {
        "schema_revision": SCHEMA_REVISION,
        "research_root": str(root),
        "source": {"path": SOURCE, "blake3": blake3_hex(manifest)},
        "cells": cells,
    }


def render(index: dict[str, Any]) -> str:
    return json.dumps(index, indent=2, sort_keys=True, ensure_ascii=True) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=DEFAULT_ROOT, help="research root (read only)")
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    args = parser.parse_args(argv)
    text = render(build_index(args.root))
    args.out.write_text(text, encoding="utf-8")
    sys.stdout.write(f"wrote {args.out} ({len(text.encode('utf-8'))} bytes)\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
