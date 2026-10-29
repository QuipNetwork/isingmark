"""Convert a frozen research model (``.npz``) into a one-instance isingmark instance file.

The model holds integer couplings in milli units on the suite's dense spin numbering:
``edges`` (E, 2), ``j_milli`` (E,), ``h_milli`` (N,), and usually ``physical_nodes`` (N,), the
qubit id of each spin. The converter carries it onto the fixture by qubit id with
``FixtureMap``, checks every value against the milli range, and writes the instance with the
model's structure arrays in ``reference``.

Node-group arrays whose entries are dense spin indices (``tetrahedra``, ``clusters``, and
``pairs`` when every pair is a model coupler) are written as fixture node ids under
``reference["id_space"] == "chain-h0"``. Every other array is written as the model stores it
under ``reference["unmapped"]`` with ``id_space`` ``"suite"``: its entries are not known to be
spin indices, so they stay untouched.
"""

from __future__ import annotations

import argparse
import json
import sys
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import numpy as np
from blake3 import blake3

from isingmark_structured.generate import (
    DEFAULT_FIXTURE,
    DEFAULT_PROBLEMS,
    FORMAT,
    MILLI_LIMIT,
    write_instances,
)
from isingmark_structured.topology import (
    DEFAULT_SUITE_SPEC,
    FixtureMap,
    SuiteSpec,
    load_fixture,
    load_suite_spec,
)

# Arrays that are the model itself, not structure.
COEFFICIENT_ARRAYS = frozenset({"edges", "j_milli", "h_milli", "physical_nodes"})
# Node-group arrays that are dense spin indices wherever they occur.
SPIN_GROUPS = frozenset({"tetrahedra", "clusters"})
INTEGRALITY_TOLERANCE = 1e-9


def blake3_hex(path: Path) -> str:
    return blake3(path.read_bytes()).hexdigest()


def to_milli(values: np.ndarray, name: str) -> np.ndarray:
    """``values`` as int64 milli units. Floats scale by 1000 and must land on an integer."""
    values = np.asarray(values)
    if values.dtype.kind in "iu":
        return values.astype(np.int64)
    if values.dtype.kind != "f":
        raise ValueError(f"{name}: dtype {values.dtype} is not numeric")
    scaled = values * 1000.0
    rounded = np.rint(scaled)
    if not np.all(np.abs(scaled - rounded) <= INTEGRALITY_TOLERANCE):
        raise ValueError(f"{name}: a value is not an integer number of milli units")
    return rounded.astype(np.int64)


def check_range(values: np.ndarray, name: str) -> None:
    if values.size and (values.min() < -MILLI_LIMIT or values.max() > MILLI_LIMIT):
        raise ValueError(
            f"{name}: values span [{values.min()}, {values.max()}], outside [-1000, 1000]"
        )


def spin_qubits(model: dict[str, np.ndarray], suite: SuiteSpec) -> np.ndarray:
    """The qubit id of every model spin. Only the suite's own numbering is supported."""
    n_spins = len(model["h_milli"])
    if n_spins != suite.n_spins:
        raise ValueError(f"the model has {n_spins} spins; the suite spec has {suite.n_spins}")
    physical = model.get("physical_nodes")
    if physical is not None and not np.array_equal(physical, suite.nodes):
        raise ValueError("physical_nodes differ from the suite spec nodes; the qubits are unmapped")
    return suite.nodes


def pairs_are_couplers(pairs: np.ndarray, edges: np.ndarray) -> bool:
    coupled = {(min(u, v), max(u, v)) for u, v in edges.tolist()}
    return all((min(u, v), max(u, v)) in coupled for u, v in pairs.tolist())


def build_reference(model: dict[str, np.ndarray], qubits: np.ndarray) -> dict[str, Any]:
    """The structure arrays of ``model``, node groups mapped to fixture node ids."""
    reference: dict[str, Any] = {"id_space": "chain-h0"}
    unmapped: dict[str, Any] = {}
    for name, array in sorted(model.items()):
        if name in COEFFICIENT_ARRAYS:
            continue
        is_group = name in SPIN_GROUPS or (
            name == "pairs" and pairs_are_couplers(array, model["edges"])
        )
        if is_group and array.dtype.kind in "iu" and array.size:
            if array.min() < 0 or array.max() >= len(qubits):
                raise ValueError(f"{name}: an entry is not a spin index")
            reference[name] = qubits[array].tolist()
        else:
            unmapped[name] = array.tolist()
    if unmapped:
        reference["unmapped"] = {"id_space": "suite", **unmapped}
    return reference


def convert(
    model_path: Path,
    cell: str,
    out: Path,
    *,
    problem_family: str,
    source_model: str,
    params: dict[str, Any] | None = None,
    verdict: str = "provisional",
    fixture_path: Path = DEFAULT_FIXTURE,
    suite_path: Path = DEFAULT_SUITE_SPEC,
) -> Path:
    """Write ``out/<cell>.jsonl`` for the model at ``model_path``. Returns the path."""
    suite = load_suite_spec(suite_path)
    fixture = load_fixture(fixture_path)
    with np.load(model_path, allow_pickle=False) as archive:
        model = {name: archive[name] for name in archive.files}
    qubits = spin_qubits(model, suite)
    j = to_milli(model["j_milli"], "j_milli")
    h = to_milli(model["h_milli"], "h_milli")
    check_range(j, "j_milli")
    check_range(h, "h_milli")
    h_fixture, j_fixture = FixtureMap(suite, fixture).map(model["edges"], j, h)
    header: dict[str, Any] = {
        "format": FORMAT,
        "family": cell,
        "problem_family": problem_family,
        "topology_hash": fixture.topology_hash(),
        "topology_fixture": fixture.name,
        "nodes": len(fixture.nodes),
        "edges": len(fixture.edges),
        "suite_topology_sha256": suite.sha256,
        "generator": "frozen-model",
        "model_blake3": blake3_hex(model_path),
        "source_model": source_model,
        "created": datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "verdict": verdict,
    }
    if params is not None:
        header["params"] = params
    instance = {
        "nonce": model_path.stem,
        "h_milli": h_fixture,
        "j_milli": j_fixture,
        "reference": build_reference(model, qubits),
    }
    out.mkdir(parents=True, exist_ok=True)
    path = out / f"{cell}.jsonl"
    write_instances(path, header, [instance])
    return path


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path, help="the frozen model .npz")
    parser.add_argument("--cell", required=True, help="cell id; names the output file")
    parser.add_argument("--problem-family", required=True, help="problems/ family slug")
    parser.add_argument(
        "--source-model", required=True, help="research-root-relative path of the model"
    )
    parser.add_argument("--out", type=Path, default=DEFAULT_PROBLEMS.parent / "corpora")
    parser.add_argument("--problems", type=Path, default=DEFAULT_PROBLEMS)
    parser.add_argument("--verdict", default="provisional")
    parser.add_argument("--fixture", type=Path, default=DEFAULT_FIXTURE)
    parser.add_argument("--suite-spec", type=Path, default=DEFAULT_SUITE_SPEC)
    args = parser.parse_args(argv)
    params_path = args.problems / args.problem_family / "params" / f"{args.cell}.json"
    params = None
    if params_path.exists():
        params = json.loads(params_path.read_text(encoding="utf-8")).get("params")
    path = convert(
        args.model,
        args.cell,
        args.out,
        problem_family=args.problem_family,
        source_model=args.source_model,
        params=params,
        verdict=args.verdict,
        fixture_path=args.fixture,
        suite_path=args.suite_spec,
    )
    print(f"{args.cell}: wrote {path}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
