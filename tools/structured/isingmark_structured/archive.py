"""Export archived research models into archived fixtures.

One file per cell goes to ``problems/<family>/fixtures/<cell>.json``.

For each selected cell the export takes the first two rows of the cell in ``targets-pp-100.json``,
reads the model ``.npz`` as the research stored it, and records digests of it: the topology order
digest (FAM-19) and the model digest over the FAM-30 instance body. The arrays stay in the
archive. Edges, h, and j are hashed as stored, never sorted or repaired. A model that does not fit
the format is rejected with an error. The witness of each instance is the postprocessed final
state when the research stored it, and its energy must equal the recorded ``pp_best``.

Format and rules: ``docs/datasets.md``, section "Archived research fixtures".
"""

from __future__ import annotations

import argparse
import json
import platform
import struct
import sys
from pathlib import Path
from typing import Any

import numpy as np
from blake3 import blake3

REPO = Path(__file__).resolve().parents[3]
DEFAULT_RESEARCH_ROOT = Path("/home/carback1/quip-data/regimes")
TARGETS = "new-pow/targets-pp-100.json"
MEASUREMENTS = "new-pow/postprocess/measurements/optimizers-100"
SCRIPT = Path(__file__).resolve()
ROWS_PER_CELL = 2
WITNESS_NAME = "pp-final"

# cell -> (problems/ family directory, generator id in problems/sources.json)
CELLS: dict[str, tuple[str, str]] = {
    "ice-charge": ("charged-ice", "f17-generate3"),
    "ice-fields-h500": ("ice-fields", "suite-ice-fields"),
    "ice-fields-h1000": ("ice-fields", "suite-ice-fields"),
    "two-commodity-routing": ("reductions", "newpow-two-commodity-routing"),
    "ice-twisted-w62-dimer1200": ("twisted-ice", "f17-generate2"),
    "ice-twisted-w62-dimer1350": ("twisted-ice", "f17-wave-run"),
    "ice-twisted-w45-dimer1200": ("twisted-ice", "f17-wave-run"),
}


TOPOLOGY_CONTEXT = "quip family topology order v1"
MODEL_CONTEXT = "isingmark archived model v1"
# FAM-30 metadata region of an instance with no sections.
EMPTY_METADATA = b"\x00\x00"
U32_MAX = 2**32 - 1
I32_MAX = 2**31 - 1


class ArchiveError(ValueError):
    """The model or its state does not fit the archived fixture format."""


def blake3_hex(data: bytes) -> str:
    return blake3(data).hexdigest()


def _u32(values: list[int]) -> bytes:
    return struct.pack(f"<{len(values)}I", *values)


def _i32(values: list[int]) -> bytes:
    return struct.pack(f"<{len(values)}i", *values)


def topology_order_digest(node_ids: list[int], edges: list[list[int]]) -> str:
    """FAM-19 over native ids in instance order. ``edges`` hold dense positions."""
    native = [node_ids[end] for edge in edges for end in edge]
    data = _u32([len(node_ids)]) + _u32(node_ids) + _u32([len(edges)]) + _u32(native)
    return blake3(data, derive_key_context=TOPOLOGY_CONTEXT).hexdigest()


def instance_body(fields: dict[str, Any], offset_milli: int = 0) -> bytes:
    """The FAM-30 body: everything after the 97-byte head, with the empty metadata region."""
    node_ids, edges = fields["node_ids"], fields["edges"]
    return b"".join(
        [
            _u32([len(node_ids)]),
            _u32(node_ids),
            _u32([len(edges)]),
            _u32([end for edge in edges for end in edge]),
            _i32(fields["h"]),
            _i32(fields["j"]),
            struct.pack("<q", offset_milli),
            EMPTY_METADATA,
        ]
    )


def model_digest(fields: dict[str, Any], offset_milli: int = 0) -> str:
    body = instance_body(fields, offset_milli)
    return blake3(body, derive_key_context=MODEL_CONTEXT).hexdigest()


def pack_spins(spins: list[int]) -> str:
    """Wire packing: spin i is bit ``i % 8`` of byte ``i // 8``, 1 for +1. Padding bits are 0."""
    packed = bytearray((len(spins) + 7) // 8)
    for index, spin in enumerate(spins):
        if spin == 1:
            packed[index // 8] |= 1 << (index % 8)
    return packed.hex()


def _int_array(model: Any, key: str, ndim: int) -> np.ndarray:
    if key not in model.files:
        raise ArchiveError(f"missing array {key}")
    try:
        values = model[key]
    except ValueError as error:
        raise ArchiveError(f"{key}: {error}") from error
    if values.dtype.kind != "i":
        raise ArchiveError(f"{key}: dtype {values.dtype} is not a signed integer")
    if values.ndim != ndim:
        raise ArchiveError(f"{key}: shape {values.shape} has {values.ndim} dimensions, not {ndim}")
    return values


def read_model(path: Path) -> dict[str, Any]:
    """Validate the NPZ at ``path`` and return its fixture fields. Nothing is sorted or repaired."""
    try:
        model = np.load(path, allow_pickle=False)
    except ValueError as error:
        raise ArchiveError(f"{path}: {error}") from error
    with model:
        nodes = _int_array(model, "physical_nodes", 1)
        h = _int_array(model, "h_milli", 1)
        edges = _int_array(model, "edges", 2)
        j = _int_array(model, "j_milli", 1)
        if edges.shape[1] != 2:
            raise ArchiveError(f"edges: shape {edges.shape} is not (E, 2)")
        if h.shape != nodes.shape:
            raise ArchiveError(
                f"h_milli: shape {h.shape} differs from physical_nodes {nodes.shape}"
            )
        if j.shape[0] != edges.shape[0]:
            raise ArchiveError(f"j_milli: shape {j.shape} differs from edges {edges.shape}")
        if nodes.size and (nodes.min() < 0 or nodes.max() > U32_MAX):
            raise ArchiveError("physical_nodes: an id does not fit u32")
        for key, values in (("h_milli", h), ("j_milli", j)):
            if values.size and np.abs(values).max() > I32_MAX:
                raise ArchiveError(f"{key}: a value does not fit i32")
        if len(set(nodes.tolist())) != nodes.size:
            raise ArchiveError("physical_nodes: duplicate node id")
        if edges.size and (edges.min() < 0 or edges.max() >= nodes.size):
            raise ArchiveError("edges: an endpoint is out of range")
        a, b = edges[:, 0], edges[:, 1]
        if np.any(a >= b):
            raise ArchiveError("edges: an edge has a >= b")
        pairs = list(zip(a.tolist(), b.tolist(), strict=True))
        for before, after in zip(pairs, pairs[1:], strict=False):
            if before >= after:
                raise ArchiveError(f"edges: unsorted or duplicate edges at {before}, {after}")
        offset = None
        if "energy_offset_quarter_milli" in model.files:
            quarter = _int_array(model, "energy_offset_quarter_milli", 0)
            if int(quarter) % 4 != 0:
                raise ArchiveError(
                    f"energy_offset_quarter_milli {int(quarter)} is not divisible by 4"
                )
            offset = str(int(quarter) // 4)
        return {
            "node_ids": nodes.tolist(),
            "edges": [list(pair) for pair in pairs],
            "h": h.tolist(),
            "j": j.tolist(),
            "qubo_offset_milli": offset,
        }


def energy_milli(fields: dict[str, Any], spins: list[int]) -> int:
    """Energy with the rescore convention: ``sum(J s_a s_b) + sum(h s)``, no offset."""
    total = sum(h * s for h, s in zip(fields["h"], spins, strict=True))
    for (a, b), j in zip(fields["edges"], fields["j"], strict=True):
        total += j * spins[a] * spins[b]
    return total


def read_state(path: Path, nodes: int) -> list[int]:
    state = np.load(path, allow_pickle=False)
    if state.dtype != np.int8 or state.shape != (nodes,):
        raise ArchiveError(f"{path}: state is {state.dtype} {state.shape}, not int8 ({nodes},)")
    if not np.all(np.abs(state) == 1):
        raise ArchiveError(f"{path}: a spin is not 1 or -1")
    return state.tolist()


def pp_best(results: Path, nonce: str) -> int:
    for line in results.read_text(encoding="utf-8").splitlines():
        row = json.loads(line)
        if row["nonce"] == nonce:
            return int(row["pp_best"])
    raise ArchiveError(f"{results}: no row for nonce {nonce}")


def witness(root: Path, family: str, cell: str, nonce: str, fields: dict[str, Any]) -> dict:
    """The ``pp-final`` witness, or a ``missing`` witness that names the path searched."""
    cell_dir = root / MEASUREMENTS / family / cell
    state_path = cell_dir / "states" / f"{nonce}.npy"
    if not state_path.is_file():
        return {"name": WITNESS_NAME, "missing": f"state not stored; searched {state_path}"}
    spins = read_state(state_path, len(fields["node_ids"]))
    recorded = pp_best(cell_dir / "results.jsonl", nonce)
    computed = energy_milli(fields, spins)
    if computed != recorded:
        raise ArchiveError(
            f"{cell} {nonce}: state energy {computed} differs from pp_best {recorded}"
        )
    return {
        "name": WITNESS_NAME,
        "spins_packed": pack_spins(spins),
        "claimed_energy_milli": str(recorded),
    }


def source_entry(repo: Path, source_id: str, sources: dict[str, dict]) -> dict[str, str]:
    path = sources[source_id]["path"]
    return {"path": path, "blake3": blake3_hex((repo / path).read_bytes())}


def build_fixture(root: Path, repo: Path, cell: str) -> dict[str, Any]:
    problem_family, source_id = CELLS[cell]
    manifest = json.loads((repo / "problems/sources.json").read_text(encoding="utf-8"))
    sources = {entry["id"]: entry for entry in manifest["sources"]}
    target = json.loads((root / TARGETS).read_text(encoding="utf-8"))[cell]
    script_hash = blake3_hex(SCRIPT.read_bytes())
    instances = []
    for index, row in enumerate(target["rows"][:ROWS_PER_CELL]):
        model_path = Path(row["model"])
        fields = read_model(model_path)
        instances.append(
            {
                "name": f"{cell}-row{index}",
                "cell": cell,
                "nonce": row["nonce"],
                "archive_origin": model_path.relative_to(root).as_posix(),
                "archive_blake3": blake3_hex(model_path.read_bytes()),
                "source_recipe": {"source": source_id, "name": cell},
                "node_order": "physical",
                "n_nodes": len(fields["node_ids"]),
                "n_edges": len(fields["edges"]),
                "topology_order_digest": topology_order_digest(fields["node_ids"], fields["edges"]),
                "model_digest": model_digest(fields),
                "offset_milli": "0",
                "qubo_offset_milli": fields["qubo_offset_milli"],
                "family_id": None,
                "descriptor_digest": None,
                "params_hex": None,
                "postprocess_policy": {
                    "family": target["family"],
                    "method": target["method"],
                    "budget_s": target["budget_s"],
                },
                "witnesses": [witness(root, target["family"], cell, row["nonce"], fields)],
            }
        )
    return {
        "schema_revision": 1,
        "provenance": "archived-research",
        "about": [
            f"Rows 0 and {ROWS_PER_CELL - 1} of {cell} in {TARGETS}, exported by "
            "tools/structured/isingmark_structured/archive.py.",
            "The arrays stay in the archive. Digests follow docs/datasets.md. The witness is the "
            "postprocessed final state, packed, and checked against pp_best.",
            f"Conversion environment: Python {platform.python_version()}, NumPy {np.__version__}, "
            f"export script blake3 {script_hash}.",
        ],
        "sources": [
            {"path": "tools/structured/isingmark_structured/archive.py", "blake3": script_hash},
            source_entry(repo, source_id, sources),
        ],
        "archived_instances": instances,
    }


def render(doc: dict[str, Any]) -> str:
    return json.dumps(doc, indent=2, sort_keys=True, ensure_ascii=True) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--research-root", type=Path, default=DEFAULT_RESEARCH_ROOT)
    parser.add_argument("--repo", type=Path, default=REPO)
    parser.add_argument("--cell", action="append", choices=sorted(CELLS), help="default: all")
    args = parser.parse_args(argv)
    for cell in args.cell or sorted(CELLS):
        try:
            text = render(build_fixture(args.research_root, args.repo, cell))
        except ArchiveError as error:
            print(f"{cell}: {error}", file=sys.stderr, flush=True)
            return 1
        out = args.repo / "problems" / CELLS[cell][0] / "fixtures" / f"{cell}.json"
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(text, encoding="utf-8")
        print(f"{cell}: wrote {out} ({len(text.encode())} bytes)", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
