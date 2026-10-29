"""Tests for the frozen-model converter, on a tiny synthetic model."""

from __future__ import annotations

import json
from pathlib import Path

import numpy as np
import pytest
from blake3 import blake3

from isingmark_structured.convert import convert, main, to_milli
from isingmark_structured.generate import read_instances

NODES = [10, 11, 12, 13, 14]
SUITE_EDGES = [[10, 11], [11, 12], [12, 13], [13, 10]]
# The fixture has one more qubit and one more coupler than the suite, in another order.
FIXTURE_NODES = [14, 13, 12, 11, 10, 15]
FIXTURE_EDGES = [[13, 10], [10, 11], [11, 12], [12, 13], [14, 15]]


def _spec(path: Path, nodes: list[int], edges: list[list[int]]) -> Path:
    path.write_text(
        json.dumps(
            {
                "nodes": nodes,
                "edges": edges,
                "allowed_h_milli": list(range(-1000, 1001)),
                "allowed_j_milli": list(range(-1000, 1001)),
            }
        )
    )
    return path


@pytest.fixture
def specs(tmp_path: Path) -> tuple[Path, Path]:
    suite = _spec(tmp_path / "suite.spec.json", NODES, SUITE_EDGES)
    fixture = _spec(tmp_path / "toy.spec.json", FIXTURE_NODES, FIXTURE_EDGES)
    return suite, fixture


def _model(tmp_path: Path, **overrides: np.ndarray) -> Path:
    arrays: dict[str, np.ndarray] = {
        "edges": np.array([[0, 1], [1, 2], [2, 3], [3, 0]]),
        "j_milli": np.array([125, -125, 1000, -1000]),
        "h_milli": np.array([0, 0, 5, 0, -7]),
        "physical_nodes": np.array(NODES),
        "tetrahedra": np.array([[0, 1, 2, 3]]),
        "pairs": np.array([[0, 1], [2, 3]]),
        "gauge": np.array([1, -1, 1, 1, -1], dtype=np.int8),
    }
    arrays.update(overrides)
    path = tmp_path / ("ab" * 32 + ".npz")
    np.savez(path, **arrays)
    return path


def _run(tmp_path: Path, specs: tuple[Path, Path], model: Path) -> Path:
    suite, fixture = specs
    return convert(
        model,
        "toy-cell",
        tmp_path / "out",
        problem_family="toy",
        source_model="cells/toy/model.npz",
        params={"k": 1},
        verdict="Closed",
        fixture_path=fixture,
        suite_path=suite,
    )


def test_converts_coefficients_into_fixture_order(tmp_path: Path, specs: tuple[Path, Path]) -> None:
    model = _model(tmp_path)
    header, instances = read_instances(_run(tmp_path, specs, model))
    assert header["generator"] == "frozen-model"
    assert header["family"] == "toy-cell"
    assert header["problem_family"] == "toy"
    assert header["model_blake3"] == blake3(model.read_bytes()).hexdigest()
    assert header["source_model"] == "cells/toy/model.npz"
    assert header["params"] == {"k": 1}
    assert header["verdict"] == "Closed"
    assert (header["nodes"], header["edges"]) == (6, 5)
    (instance,) = instances
    assert instance["nonce"] == "ab" * 32
    # Fixture nodes are 14, 13, 12, 11, 10, 15; fixture edges are in the order above.
    assert instance["h_milli"] == [-7, 0, 5, 0, 0, 0]
    assert instance["j_milli"] == [-1000, 125, -125, 1000, 0]


def test_reference_maps_node_groups_and_keeps_the_rest(
    tmp_path: Path, specs: tuple[Path, Path]
) -> None:
    _, (instance,) = read_instances(_run(tmp_path, specs, _model(tmp_path)))
    reference = instance["reference"]
    assert reference["id_space"] == "chain-h0"
    assert reference["tetrahedra"] == [[10, 11, 12, 13]]
    assert reference["pairs"] == [[10, 11], [12, 13]]
    assert reference["unmapped"] == {"id_space": "suite", "gauge": [1, -1, 1, 1, -1]}
    assert "j_milli" not in reference
    assert "physical_nodes" not in reference


def test_pairs_that_are_not_couplers_stay_in_the_suite_space(
    tmp_path: Path, specs: tuple[Path, Path]
) -> None:
    model = _model(tmp_path, pairs=np.array([[0, 2]]))
    _, (instance,) = read_instances(_run(tmp_path, specs, model))
    assert "pairs" not in instance["reference"]
    assert instance["reference"]["unmapped"]["pairs"] == [[0, 2]]


def test_float_coefficients_scale_to_milli() -> None:
    assert to_milli(np.array([0.125, -1.0, 0.0]), "j").tolist() == [125, -1000, 0]


def test_float_coefficients_must_be_integral_in_milli() -> None:
    with pytest.raises(ValueError, match="not an integer"):
        to_milli(np.array([0.1235]), "j")


def test_float_model_converts(tmp_path: Path, specs: tuple[Path, Path]) -> None:
    model = _model(tmp_path, j_milli=np.array([0.125, -0.125, 1.0, -1.0]))
    _, (instance,) = read_instances(_run(tmp_path, specs, model))
    assert instance["j_milli"] == [-1000, 125, -125, 1000, 0]


def test_out_of_range_value_fails_without_clamping(
    tmp_path: Path, specs: tuple[Path, Path]
) -> None:
    model = _model(tmp_path, j_milli=np.array([125, -125, 1001, 0]))
    with pytest.raises(ValueError, match="outside"):
        _run(tmp_path, specs, model)
    assert not (tmp_path / "out" / "toy-cell.jsonl").exists()


def test_unmapped_qubits_fail(tmp_path: Path, specs: tuple[Path, Path]) -> None:
    model = _model(tmp_path, physical_nodes=np.array([10, 11, 12, 13, 99]))
    with pytest.raises(ValueError, match="unmapped"):
        _run(tmp_path, specs, model)


def test_model_on_another_graph_fails(tmp_path: Path, specs: tuple[Path, Path]) -> None:
    model = _model(
        tmp_path,
        edges=np.array([[0, 1]]),
        j_milli=np.array([1]),
        h_milli=np.zeros(2, dtype=np.int64),
        physical_nodes=np.array([0, 1]),
    )
    with pytest.raises(ValueError, match="spins"):
        _run(tmp_path, specs, model)


def test_cli_reads_params_from_the_problems_tree(tmp_path: Path, specs: tuple[Path, Path]) -> None:
    suite, fixture = specs
    params = tmp_path / "problems" / "toy" / "params" / "toy-cell.json"
    params.parent.mkdir(parents=True)
    params.write_text(json.dumps({"params": {"background_milli": 125}}))
    main(
        [
            str(_model(tmp_path)),
            "--cell", "toy-cell",
            "--problem-family", "toy",
            "--source-model", "x.npz",
            "--out", str(tmp_path / "out"),
            "--problems", str(tmp_path / "problems"),
            "--fixture", str(fixture),
            "--suite-spec", str(suite),
        ]
    )  # fmt: skip
    header, _ = read_instances(tmp_path / "out" / "toy-cell.jsonl")
    assert header["params"] == {"background_milli": 125}
