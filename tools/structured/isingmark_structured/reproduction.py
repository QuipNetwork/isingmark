"""Write the archived models of one cell as campaign instances, for a reproduction control arm.

The control arm runs the reproduction policies on the archived models themselves, so each
classical energy pairs with the archived ``qpu80_pp`` of the same model. The models come from
``evaluation/qblock/archive-index.json``: every row names a model ``.npz`` under the research
root and the BLAKE3 of that file. The command checks each file against its digest, reads the
model without sorting or repairing it, and writes one ``ising-fixed`` schema 2 instance per row.

Each instance sets ``objective.max_energy`` to ``(qpu80_pp + 1) / 1000``. The campaign accepts
an energy strictly below the threshold, so a trial is accepted when a read reaches ``qpu80_pp``,
ties included. The command also writes ``manifest.json`` from a policy template and
``control.json`` with the rows it used.

    python -m isingmark_structured.reproduction --cell ice-charge \\
        --policies evaluation/qblock/configs/reproduction/ice-charge.policies.json --out DIR
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

from blake3 import blake3

from .archive import DEFAULT_RESEARCH_ROOT, REPO, ArchiveError, read_model

DEFAULT_INDEX = REPO / "evaluation/qblock/archive-index.json"


def file_blake3(path: Path) -> str:
    hasher = blake3()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            hasher.update(block)
    return hasher.hexdigest()


def natural(milli: int) -> float:
    """``milli / 1000``. Python writes this float as the exact decimal, for example ``-0.031``."""
    return milli / 1000


def instance(cell: str, row: dict[str, Any], model: dict[str, Any]) -> dict[str, Any]:
    if model["qubo_offset_milli"] is not None:
        raise ArchiveError(f"{row['nonce']}: a model with an energy offset is not supported")
    return {
        "schema_version": 2,
        "instance_id": f"{cell}-a-{row['nonce'][:16]}",
        "family": "ising-fixed",
        "model_version": "1",
        "challenge_id": f"control-{cell}",
        "nonce": row["nonce"],
        "split": "validation",
        "physical": {
            "variables": [f"n{node}" for node in model["node_ids"]],
            "h": [natural(value) for value in model["h"]],
            "j": [natural(value) for value in model["j"]],
            "edges": model["edges"],
            "offset": 0,
            "auxiliaries": [],
            "precision_bound": 0.0,
            "exact": True,
        },
        "search_space": {},
        "objective": {"max_energy": natural(int(row["qpu80_pp"]) + 1)},
        "constraints": {},
        "hardness": {},
        "source_hashes": {"model": row["model_blake3"]},
    }


def write_control(
    cell: str, index_path: Path, root: Path, policies: Path, out: Path
) -> dict[str, Any]:
    if out.exists():
        raise ArchiveError(f"{out} exists; the control is written only into a new directory")
    index = json.loads(index_path.read_text(encoding="utf-8"))
    cells = [entry for entry in index["cells"] if entry["cell"] == cell]
    if len(cells) != 1:
        raise ArchiveError(f"{index_path} has {len(cells)} entries for {cell}")
    manifest = json.loads(policies.read_text(encoding="utf-8"))
    if "instances" in manifest:
        raise ArchiveError("the policy template must not list instances")
    out.mkdir(parents=True)
    rows = []
    for row in cells[0]["rows"]:
        path = root / row["model"]
        digest = file_blake3(path)
        if digest != row["model_blake3"]:
            raise ArchiveError(f"{path}: BLAKE3 {digest} differs from the index")
        body = instance(cell, row, read_model(path))
        name = f"{body['instance_id']}.instance.json"
        (out / name).write_text(json.dumps(body, separators=(",", ":")) + "\n", encoding="utf-8")
        rows.append(
            {
                "instance_id": body["instance_id"],
                "nonce": row["nonce"],
                "model": row["model"],
                "model_blake3": digest,
                "qpu80_pp": row["qpu80_pp"],
                "file": name,
            }
        )
    manifest["instances"] = [row["file"] for row in rows]
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    record = {
        "cell": cell,
        "index": str(index_path),
        "index_blake3": file_blake3(index_path),
        "research_root": str(root),
        "max_energy_rule": "(qpu80_pp + 1) / 1000",
        "rows": rows,
    }
    (out / "control.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    return record


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cell", required=True)
    parser.add_argument("--policies", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--index", type=Path, default=DEFAULT_INDEX)
    parser.add_argument("--research-root", type=Path, default=DEFAULT_RESEARCH_ROOT)
    args = parser.parse_args(argv)
    try:
        record = write_control(args.cell, args.index, args.research_root, args.policies, args.out)
    except ArchiveError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(f"{len(record['rows'])} instances in {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
