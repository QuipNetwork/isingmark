"""Tests for the nonce list, the draw, the topology hash, the fixture map and the writer."""

from __future__ import annotations

import json
from pathlib import Path

import numpy as np
import pytest

from isingmark_structured import protocol
from isingmark_structured.generate import (
    DEFAULT_FIXTURE,
    DEFAULT_OUT,
    build,
    params_file,
    read_instances,
    write_instances,
)
from isingmark_structured.topology import (
    DEFAULT_SUITE_SPEC,
    Fixture,
    FixtureMap,
    load_suite_spec,
)

ROOT = Path(__file__).resolve().parents[3]
SUITE_MANIFEST = Path(
    "/home/carback1/quip-data/regimes/qpu-suite/work/cells/ice-glass-r8/manifest.json"
)
COMMITTED_MANIFEST = ROOT / "problems" / "ice-glass" / "params" / "ice-glass-r8.json"


def test_nonce_list_matches_the_suite() -> None:
    source = SUITE_MANIFEST if SUITE_MANIFEST.exists() else COMMITTED_MANIFEST
    raw = json.loads(source.read_text())
    recorded = [row["nonce"] for row in raw["models"]] if "models" in raw else raw["nonces"]
    assert protocol.make_nonces(3) == recorded[:3]


def test_keystream_matches_the_chacha8_block_function() -> None:
    # Word 16 starts block 1; a draw across the boundary equals two separate draws.
    nonce = bytes(range(32))
    whole = protocol.keystream_words(nonce, 10, 20)
    assert np.array_equal(whole[:6], protocol.keystream_words(nonce, 10, 6))
    assert np.array_equal(whole[6:], protocol.keystream_words(nonce, 16, 14))


def test_draw_selects_word_modulo_alphabet() -> None:
    nonce = bytes([0x77] * 32)
    h, j = protocol.draw_ising_milli(nonce, 37, 101, [-1000, 0, 1000], [-1000, 1000])
    words = protocol.keystream_words(nonce, 0, 138).astype(np.int64)
    assert np.array_equal(h, np.asarray([-1000, 0, 1000])[words[:37] % 3])
    assert np.array_equal(j, np.asarray([-1000, 1000])[words[37:] % 2])


def test_topology_hash_matches_the_upstream_golden_vector() -> None:
    fixture = Fixture(
        "golden", [0, 1, 2], [(0, 1), (1, 2)], [-1000, 0, 1000], [-1000, 1000], [-1000, 1000]
    )
    assert (
        fixture.topology_hash()
        == "56dff5ca824517ba7f0593ec12a5dd102eb0a14f775b35c415467e0e0d6e19c2"
    )


def test_chain_h0_fixture_hashes_to_its_bucket() -> None:
    from isingmark_structured.topology import load_fixture

    fixture = load_fixture(ROOT / "fixtures" / "chain-h0.spec.json")
    assert (
        fixture.topology_hash()
        == "e66d3dfa3c9c6afb15efe29891cf9412498d94692ab5956af3ad98ba3693a02d"
    )


def _suite(tmp_path: Path, nodes: list[int], edges: list[list[int]]) -> Path:
    path = tmp_path / "suite.spec.json"
    path.write_text(
        json.dumps(
            {
                "nodes": nodes,
                "edges": edges,
                "allowed_h_milli": [0],
                "allowed_j_milli": [-1000, 1000],
            }
        )
    )
    return path


def test_fixture_map_places_values_by_qubit_id(tmp_path: Path) -> None:
    # The suite lists qubits 7, 3, 5; the fixture holds them in another order plus qubit 9.
    suite = load_suite_spec(_suite(tmp_path, [7, 3, 5], [[7, 3], [5, 3]]))
    fixture = Fixture(
        "tiny", [3, 5, 7, 9], [(9, 3), (3, 5), (3, 7), (5, 7)], [0], [-1000, 1000], [-1000, 1000]
    )
    mapping = FixtureMap(suite, fixture)
    # Dense spins: 0 is qubit 7, 1 is qubit 3, 2 is qubit 5.
    h, j = mapping.map(
        np.asarray([[0, 1], [2, 1]]), np.asarray([125, -1000]), np.asarray([10, 20, 30])
    )
    assert h == [20, 30, 10, 0]
    assert j == [0, -1000, 125, 0]


def test_fixture_map_rejects_a_missing_edge(tmp_path: Path) -> None:
    suite = load_suite_spec(_suite(tmp_path, [1, 2, 3], [[1, 2], [2, 3]]))
    fixture = Fixture("tiny", [1, 2, 3], [(1, 2)], [0], [-1000, 1000], [-1000, 1000])
    with pytest.raises(ValueError, match="lacks 1 couplers"):
        FixtureMap(suite, fixture)


def test_fixture_map_rejects_a_missing_node(tmp_path: Path) -> None:
    suite = load_suite_spec(_suite(tmp_path, [1, 2], [[1, 2]]))
    fixture = Fixture("tiny", [1, 3], [(1, 3)], [0], [-1000, 1000], [-1000, 1000])
    with pytest.raises(ValueError, match="lacks 1 qubits"):
        FixtureMap(suite, fixture)


def test_writer_round_trip(tmp_path: Path) -> None:
    header = {"format": 1, "family": "tiny", "topology_hash": "00" * 32, "nodes": 2, "edges": 1}
    instances = [
        {"nonce": "ab" * 32, "h_milli": [0, 0], "j_milli": [125]},
        {
            "nonce": "cd" * 32,
            "h_milli": [0, 0],
            "j_milli": [-1000],
            "reference": {"qpu_best_milli": -1000},
        },
    ]
    path = tmp_path / "tiny.jsonl"
    assert write_instances(path, header, instances) == 2
    read_header, read_back = read_instances(path)
    assert read_header == {"kind": "header", **header}
    assert read_back == instances


def test_writer_rejects_an_out_of_range_value(tmp_path: Path) -> None:
    header = {"format": 1, "family": "tiny"}
    with pytest.raises(ValueError, match="outside"):
        write_instances(tmp_path / "bad.jsonl", header, [{"nonce": "ab" * 32, "j_milli": [1001]}])


def _params(problems: Path, family: str, cell: str) -> Path:
    path = problems / family / "params" / f"{cell}.json"
    path.parent.mkdir(parents=True)
    path.write_text("{}")
    return path


def test_params_file_finds_the_single_match(tmp_path: Path) -> None:
    expected = _params(tmp_path, "ice-glass", "ice-glass-r8")
    assert params_file(tmp_path, "ice-glass-r8", None) == expected


def test_params_file_honors_an_explicit_family(tmp_path: Path) -> None:
    _params(tmp_path, "ice-glass", "ice-glass-r8")
    chosen = tmp_path / "other" / "params" / "ice-glass-r8.json"
    assert params_file(tmp_path, "ice-glass-r8", "other") == chosen


def test_params_file_rejects_two_matches(tmp_path: Path) -> None:
    _params(tmp_path, "a", "cell")
    _params(tmp_path, "b", "cell")
    with pytest.raises(SystemExit, match="several"):
        params_file(tmp_path, "cell", None)
    with pytest.raises(SystemExit, match="pass --problem-family"):
        params_file(tmp_path, "missing", None)


def test_default_out_is_outside_the_tracked_examples() -> None:
    assert DEFAULT_OUT == ROOT / "out" / "structured"


def _build(out: Path, params: Path, update: bool) -> None:
    build(
        "ice-glass-r8",
        out,
        params,
        1,
        load_suite_spec(DEFAULT_SUITE_SPEC),
        DEFAULT_FIXTURE,
        None,
        {},
        update,
    )


def test_build_writes_params_without_tier_or_verdict(tmp_path: Path) -> None:
    params = tmp_path / "p" / "params" / "ice-glass-r8.json"
    _build(tmp_path / "out", params, False)
    written = json.loads(params.read_text())
    assert "tier" not in written
    assert "verdict" not in written
    assert (tmp_path / "out" / "ice-glass-r8.jsonl").exists()


def test_build_keeps_existing_params_unless_asked(tmp_path: Path) -> None:
    params = tmp_path / "ice-glass-r8.json"
    nonce = "ab" * 32
    params.write_text(json.dumps({"nonces": [nonce]}))
    _build(tmp_path / "out", params, False)
    assert json.loads(params.read_text()) == {"nonces": [nonce]}
    _, instances = read_instances(tmp_path / "out" / "ice-glass-r8.jsonl")
    assert [row["nonce"] for row in instances] == [nonce]
    _build(tmp_path / "out", params, True)
    assert "port_sha256" in json.loads(params.read_text())
