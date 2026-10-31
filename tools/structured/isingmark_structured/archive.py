"""Export archived research models into archived fixtures.

One file per cell goes to ``problems/<family>/fixtures/<cell>.json``.

For each selected cell the export takes the first two rows of the cell in ``targets-pp-100.json``,
reads the model ``.npz`` as the research stored it, and records digests of it: the topology order
digest (FAM-19) and the model digest over the FAM-30 instance body. The arrays stay in the
archive. Edges, h, and j are hashed as stored, never sorted or repaired. A model that does not fit
the format is rejected with an error.

Each instance has up to three witnesses: ``qpu-best`` (the lowest raw QPU read), ``greedy`` (the
state that ``qpu80_greedy`` came from), and ``pp-final`` (the postprocessed final state). Each
spins witness has an ``origin`` with the archive file path, its BLAKE3, the array key (or null),
and the read index (or null). The energy of each state is recomputed from the model and must
equal the recorded ``qpu80_best``, ``qpu80_greedy``, or ``pp_best``. The export stops on any
difference.

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
GREEDY = "new-pow/postprocess/measurements/greedy-100"
OPTIMIZERS = "new-pow/postprocess/optimizers"
# Raw QPU reads live at <root>/<cell>/qpu-80/<nonce>.npz, first root that has the file
# (the order of ARCHIVE_ROOTS in new-pow/postprocess/descent.py).
QPU_ROOTS = ("new-pow/qpu-recap/qpu", "qpu-f17/qpu", "qpu-suite/qpu")
SCRIPT = Path(__file__).resolve()
ROWS_PER_CELL = 2
PP_WITNESS = "pp-final"
QPU_WITNESS = "qpu-best"
GREEDY_WITNESS = "greedy"

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
I32_MIN = -(2**31)
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
            if values.size and (values.min() < I32_MIN or values.max() > I32_MAX):
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


def pp_best(results: Path, nonce: str) -> int:
    for line in results.read_text(encoding="utf-8").splitlines():
        row = json.loads(line)
        if row["nonce"] == nonce:
            return int(row["pp_best"])
    raise ArchiveError(f"{results}: no row for nonce {nonce}")


def check_states(states: np.ndarray, nodes: int, label: str, ndim: int = 2) -> np.ndarray:
    """``states`` as an int8 array of +1 and -1 with ``ndim`` dimensions and ``nodes`` columns."""
    if states.dtype != np.int8 or states.ndim != ndim or states.shape[-1] != nodes:
        raise ArchiveError(
            f"{label}: states are {states.dtype} {states.shape}, not int8 with {ndim} "
            f"dimensions ending in {nodes}"
        )
    if not np.all(np.abs(states) == 1):
        raise ArchiveError(f"{label}: a spin is not 1 or -1")
    return states


def energies_milli(fields: dict[str, Any], states: np.ndarray) -> np.ndarray:
    """Rescore energies of every row of ``states`` (shape ``(reads, nodes)``), no offset."""
    spins = states.astype(np.int64)
    edges = np.asarray(fields["edges"], dtype=np.int64).reshape(-1, 2)
    h = np.asarray(fields["h"], dtype=np.int64)
    j = np.asarray(fields["j"], dtype=np.int64)
    pair = spins[:, edges[:, 0]] * spins[:, edges[:, 1]]
    return spins @ h + pair @ j


def load_array(path: Path, array: str | None) -> np.ndarray:
    """An array of an NPZ, or the whole ``.npy`` when ``array`` is None. No pickles."""
    try:
        loaded = np.load(path, allow_pickle=False)
        if array is None:
            return loaded
        with loaded:
            return loaded[array]
    except (ValueError, KeyError, OSError) as error:
        raise ArchiveError(f"{path}: {array}: {error}") from error


def origin(root: Path, path: Path, array: str | None, index: int | None) -> dict[str, Any]:
    return {
        "path": path.relative_to(root).as_posix(),
        "blake3": blake3_hex(path.read_bytes()),
        "array": array,
        "index": index,
    }


def spins_witness(
    name: str, fields: dict[str, Any], state: np.ndarray, expected: int, source: dict[str, Any]
) -> dict[str, Any]:
    """A witness whose energy, recomputed from the model, must equal ``expected``."""
    computed = int(energies_milli(fields, state[None, :])[0])
    if computed != expected:
        raise ArchiveError(f"{name}: state energy {computed} differs from the recorded {expected}")
    return {
        "name": name,
        "spins_packed": pack_spins(state.tolist()),
        "claimed_energy_milli": str(expected),
        "origin": source,
    }


def missing_witness(name: str, reason: str) -> dict[str, str]:
    return {"name": name, "missing": reason}


def qpu_read(
    root: Path, cell: str, nonce: str, nodes: int
) -> tuple[Path, np.ndarray, int] | list[Path]:
    """The raw QPU file, its states, and the index of the lowest energy read.

    Returns the list of searched paths when no root has the file.
    """
    searched = [root / base / cell / "qpu-80" / f"{nonce}.npz" for base in QPU_ROOTS]
    path = next((p for p in searched if p.is_file()), None)
    if path is None:
        return searched
    if bool(np.any(load_array(path, "mock"))):
        raise ArchiveError(f"{path}: the reads are mock data")
    states = check_states(load_array(path, "spins"), nodes, str(path))
    energies = load_array(path, "energies")
    if energies.shape != (states.shape[0],):
        raise ArchiveError(f"{path}: energies {energies.shape} do not match spins {states.shape}")
    return path, states, int(np.argmin(energies))


def qpu_witness(
    root: Path, cell: str, row: dict[str, Any], fields: dict[str, Any], read: Any
) -> dict[str, Any]:
    if isinstance(read, list):
        searched = ", ".join(p.relative_to(root).as_posix() for p in read)
        return missing_witness(QPU_WITNESS, f"reads not stored, searched {searched}")
    path, states, best = read
    source = origin(root, path, "spins", best)
    return spins_witness(QPU_WITNESS, fields, states[best], int(row["qpu80_best"]), source)


def greedy_witness(
    root: Path, cell: str, row: dict[str, Any], fields: dict[str, Any], read: Any
) -> dict[str, Any]:
    """The greedy state that the recorded ``qpu80_greedy`` came from.

    Every cell has ``gnat_c`` and takes its lowest energy read. Ice cells also store
    ``loop_strict``, and take its state for the read with the lowest raw energy when that state is
    at least as low as the best ``gnat_c`` state.
    """
    nonce, nodes = row["nonce"], len(fields["node_ids"])
    path = root / GREEDY / cell / f"{nonce}.states.npz"
    if not path.is_file():
        return missing_witness(
            GREEDY_WITNESS, f"states not stored, searched {path.relative_to(root).as_posix()}"
        )
    gnat = check_states(load_array(path, "gnat_c"), nodes, f"{path}: gnat_c")
    with np.load(path, allow_pickle=False) as stored:
        has_loop = "loop_strict" in stored.files
    loop = np.zeros((0, nodes), dtype=np.int8)
    if has_loop:
        loop = check_states(load_array(path, "loop_strict"), nodes, f"{path}: loop_strict")
    gnat_best = int(np.argmin(energies_milli(fields, gnat)))
    chosen, array, index = gnat[gnat_best], "gnat_c", gnat_best
    if loop.shape[0] > 0:
        if isinstance(read, list):
            raise ArchiveError(
                f"{path}: loop_strict has rows but the raw QPU reads are not stored, so the "
                "greedy rule cannot be applied"
            )
        raw_best = read[2]
        if raw_best >= loop.shape[0]:
            raise ArchiveError(f"{path}: loop_strict has no row {raw_best}")
        loop_energy = int(energies_milli(fields, loop[raw_best : raw_best + 1])[0])
        gnat_energy = int(energies_milli(fields, gnat[gnat_best : gnat_best + 1])[0])
        if loop_energy <= gnat_energy:
            chosen, array, index = loop[raw_best], "loop_strict", raw_best
    source = origin(root, path, array, index)
    return spins_witness(GREEDY_WITNESS, fields, chosen, int(row["qpu80_greedy"]), source)


def pp_witness(
    root: Path, family: str, cell: str, nonce: str, fields: dict[str, Any]
) -> dict[str, Any]:
    """The ``pp-final`` witness, or a ``missing`` witness that names the path searched."""
    cell_dir = root / MEASUREMENTS / family / cell
    state_path = cell_dir / "states" / f"{nonce}.npy"
    if not state_path.is_file():
        return missing_witness(
            PP_WITNESS, f"state not stored, searched {state_path.relative_to(root).as_posix()}"
        )
    nodes = len(fields["node_ids"])
    state = check_states(load_array(state_path, None), nodes, str(state_path), ndim=1)
    recorded = pp_best(cell_dir / "results.jsonl", nonce)
    return spins_witness(PP_WITNESS, fields, state, recorded, origin(root, state_path, None, None))


def witnesses(
    root: Path, target: dict[str, Any], cell: str, row: dict[str, Any], fields: dict[str, Any]
) -> list[dict[str, Any]]:
    read = qpu_read(root, cell, row["nonce"], len(fields["node_ids"]))
    return [
        qpu_witness(root, cell, row, fields, read),
        greedy_witness(root, cell, row, fields, read),
        pp_witness(root, target["family"], cell, row["nonce"], fields),
    ]


def policy_sources(root: Path, family: str) -> list[dict[str, str]]:
    """Every ``*.py`` file directly in the optimizer directory of ``family``, sorted by path."""
    directory = root / OPTIMIZERS / family
    files = sorted(p for p in directory.glob("*.py") if p.is_file())
    if not files:
        raise ArchiveError(f"{directory}: no optimizer sources")
    return [
        {"path": p.relative_to(root).as_posix(), "blake3": blake3_hex(p.read_bytes())}
        for p in files
    ]


def source_entry(repo: Path, source_id: str, sources: dict[str, dict]) -> dict[str, str]:
    path = sources[source_id]["path"]
    return {"path": path, "blake3": blake3_hex((repo / path).read_bytes())}


def build_fixture(root: Path, repo: Path, cell: str) -> dict[str, Any]:
    problem_family, source_id = CELLS[cell]
    manifest = json.loads((repo / "problems/sources.json").read_text(encoding="utf-8"))
    sources = {entry["id"]: entry for entry in manifest["sources"]}
    target = json.loads((root / TARGETS).read_text(encoding="utf-8"))[cell]
    script_hash = blake3_hex(SCRIPT.read_bytes())
    sources_py = policy_sources(root, target["family"])
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
                    "sources": sources_py,
                },
                "witnesses": witnesses(root, target, cell, row, fields),
            }
        )
    return {
        "schema_revision": 1,
        "provenance": "archived-research",
        "about": [
            f"Rows 0 and {ROWS_PER_CELL - 1} of {cell} in {TARGETS}, exported by "
            "tools/structured/isingmark_structured/archive.py.",
            "The arrays stay in the archive. Digests follow docs/datasets.md. Every witness is an "
            "archived observation that was shown before: the best raw QPU read, the greedy "
            "state, and the postprocessed final state. Each energy was recomputed from the "
            "model and equals the recorded value.",
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
