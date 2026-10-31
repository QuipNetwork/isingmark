"""Tests for the archived-model export, on a tiny synthetic NPZ."""

from __future__ import annotations

import json
from pathlib import Path

import numpy as np
import pytest
from blake3 import blake3

from isingmark_structured import archive
from isingmark_structured.archive import ArchiveError, energy_milli, read_model, render

NODES = [907, 41, 12, 5]
EDGES = [[0, 1], [0, 2], [1, 3]]
J = [-1000, 62, 125]
H = [0, -500, 31, 7]


def _write(path: Path, **overrides: object) -> Path:
    arrays: dict[str, object] = {
        "physical_nodes": np.array(NODES, dtype=np.int64),
        "h_milli": np.array(H, dtype=np.int64),
        "edges": np.array(EDGES, dtype=np.int64),
        "j_milli": np.array(J, dtype=np.int64),
    }
    arrays.update(overrides)
    np.savez(path, **arrays)
    return path


def test_round_trip_copies_values_unchanged(tmp_path: Path) -> None:
    fields = read_model(_write(tmp_path / "m.npz"))
    assert fields == {
        "node_ids": NODES,
        "edges": EDGES,
        "h": H,
        "j": J,
        "qubo_offset_milli": None,
    }


def test_energy_follows_rescore_convention() -> None:
    fields = {"edges": EDGES, "h": H, "j": J}
    spins = [1, -1, 1, -1]
    expected = (0 + 500 + 31 - 7) + (-1000 * -1 + 62 * 1 + 125 * 1)
    assert energy_milli(fields, spins) == expected


def test_routing_offset_is_divided_by_four(tmp_path: Path) -> None:
    path = _write(tmp_path / "m.npz", energy_offset_quarter_milli=np.array(1787584))
    assert read_model(path)["qubo_offset_milli"] == "446896"


def test_routing_offset_not_divisible_by_four_is_rejected(tmp_path: Path) -> None:
    path = _write(tmp_path / "m.npz", energy_offset_quarter_milli=np.array(1787585))
    with pytest.raises(ArchiveError, match="divisible by 4"):
        read_model(path)


@pytest.mark.parametrize(
    ("overrides", "message"),
    [
        ({"edges": np.array([EDGES[0], EDGES[1], [1, 4]], dtype=np.int64)}, "out of range"),
        ({"edges": np.array([[0, 1], [0, 2], [3, 1]], dtype=np.int64)}, "a >= b"),
        ({"edges": np.array([[0, 2], [0, 1], [1, 3]], dtype=np.int64)}, "unsorted"),
        ({"edges": np.array([[0, 1], [0, 1], [1, 3]], dtype=np.int64)}, "duplicate"),
        ({"h_milli": np.array(H, dtype=np.float64)}, "dtype"),
        ({"j_milli": np.array(J[:2], dtype=np.int64)}, "differs from edges"),
        ({"h_milli": np.array(H[:3], dtype=np.int64)}, "differs from physical_nodes"),
        ({"edges": np.array([0, 1, 2], dtype=np.int64)}, "dimensions"),
        ({"physical_nodes": np.array(NODES + [5], dtype=np.int64)}, "differs from"),
        ({"physical_nodes": np.array([907, 41, 12, 12], dtype=np.int64)}, "duplicate node"),
    ],
)
def test_malformed_model_is_rejected(
    tmp_path: Path, overrides: dict[str, object], message: str
) -> None:
    with pytest.raises(ArchiveError, match=message):
        read_model(_write(tmp_path / "m.npz", **overrides))


def test_object_array_is_rejected(tmp_path: Path) -> None:
    ragged = np.empty(3, dtype=object)
    ragged[:] = [[0, 1], [0, 2], [1, 3]]
    with pytest.raises(ArchiveError, match="edges"):
        read_model(_write(tmp_path / "m.npz", edges=ragged))


def test_missing_array_is_rejected(tmp_path: Path) -> None:
    path = tmp_path / "m.npz"
    np.savez(path, physical_nodes=np.array(NODES))
    with pytest.raises(ArchiveError, match="missing array"):
        read_model(path)


def _research(tmp_path: Path, *, state: list[int] | None, pp_best: int) -> tuple[Path, Path, str]:
    root = tmp_path / "root"
    nonce = "ab" * 32
    (root / "models").mkdir(parents=True)
    model = _write(root / "models" / f"{nonce}.npz")
    cell_dir = root / archive.MEASUREMENTS / "ice-tiles" / "toy"
    (cell_dir / "states").mkdir(parents=True)
    (cell_dir / "results.jsonl").write_text(json.dumps({"nonce": nonce, "pp_best": pp_best}) + "\n")
    if state is not None:
        np.save(cell_dir / "states" / f"{nonce}.npy", np.array(state, dtype=np.int8))
    targets = {
        "toy": {
            "family": "ice-tiles",
            "method": "anneal",
            "budget_s": 0.25,
            "rows": [{"nonce": nonce, "model": str(model)}] * 2,
        }
    }
    (root / "new-pow").mkdir(exist_ok=True)
    (root / archive.TARGETS).write_text(json.dumps(targets))
    return root, model, nonce


def _repo(tmp_path: Path) -> Path:
    repo = tmp_path / "repo"
    generator = repo / "gen.py"
    generator.parent.mkdir()
    generator.write_text("print()\n")
    (repo / "problems").mkdir()
    manifest = {"sources": [{"id": "toy-gen", "path": "gen.py"}]}
    (repo / "problems/sources.json").write_text(json.dumps(manifest))
    return repo


def test_fixture_round_trip(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    spins = [1, -1, 1, -1]
    root, model, nonce = _research(tmp_path, state=spins, pp_best=1711)
    monkeypatch.setitem(archive.CELLS, "toy", ("toy-family", "toy-gen"))
    doc = archive.build_fixture(root, _repo(tmp_path), "toy")
    first = doc["archived_instances"][0]
    assert [i["name"] for i in doc["archived_instances"]] == ["toy-row0", "toy-row1"]
    assert first["archive_blake3"] == blake3(model.read_bytes()).hexdigest()
    assert first["archive_origin"] == f"models/{nonce}.npz"
    assert first["witnesses"] == [
        {"name": "pp-final", "spins_packed": "05", "claimed_energy_milli": "1711"}
    ]
    assert first["postprocess_policy"] == {
        "family": "ice-tiles",
        "method": "anneal",
        "budget_s": 0.25,
    }
    assert (first["n_nodes"], first["n_edges"]) == (4, 3)
    assert "edges" not in first
    assert len(first["model_digest"]) == 64
    assert doc["sources"][1]["path"] == "gen.py"
    assert render(doc).endswith("}\n")


def test_missing_state_is_recorded_with_the_searched_path(tmp_path: Path) -> None:
    root, _, nonce = _research(tmp_path, state=None, pp_best=0)
    fields = read_model(_write(tmp_path / "m.npz"))
    entry = archive.witness(root, "ice-tiles", "toy", nonce, fields)
    assert entry["name"] == "pp-final"
    assert f"states/{nonce}.npy" in entry["missing"]


def test_energy_mismatch_is_rejected(tmp_path: Path) -> None:
    root, _, nonce = _research(tmp_path, state=[1, -1, 1, -1], pp_best=-1)
    fields = read_model(_write(tmp_path / "m.npz"))
    with pytest.raises(ArchiveError, match="differs from pp_best"):
        archive.witness(root, "ice-tiles", "toy", nonce, fields)


def test_bad_state_is_rejected(tmp_path: Path) -> None:
    root, _, nonce = _research(tmp_path, state=[1, -1, 0, -1], pp_best=0)
    fields = read_model(_write(tmp_path / "m.npz"))
    with pytest.raises(ArchiveError, match="not 1 or -1"):
        archive.witness(root, "ice-tiles", "toy", nonce, fields)


def test_blake3_matches_known_value() -> None:
    assert archive.blake3_hex(b"").startswith("af1349b9f5f9a1a6")


# Pinned from the Rust upstream path (ising-generators 0.0.4) for the same synthetic instance as
# `pinning::digests_match_the_pinned_values` in tests/campaign_import_fixtures.rs.
PIN_TOPOLOGY_ORDER_DIGEST = "c30b0cb2b13ee310c3b6c896c68fd62641ae0221f70353af055b38a0f2e13a78"
PIN_MODEL_DIGEST = "c421554f0a6d5d9e1df450809473ab23542b9c3b0ca1188803f05d641e9c720f"
PIN_INSTANCE_DIGEST = "f20d5158084abd1651b738cfc304e4be7c54239553d178964feac0b6874956c5"
PIN_FIELDS = {
    "node_ids": [907, 41, 12],
    "edges": [[0, 1], [0, 2]],
    "h": [0, -500, 31],
    "j": [-1000, 62],
}


def test_digests_match_the_rust_pins() -> None:
    topology = archive.topology_order_digest(PIN_FIELDS["node_ids"], PIN_FIELDS["edges"])
    assert topology == PIN_TOPOLOGY_ORDER_DIGEST
    assert archive.model_digest(PIN_FIELDS, offset_milli=-5) == PIN_MODEL_DIGEST
    # FAM-18: head (format byte, descriptor digest, topology order digest, nonce) plus body.
    head = b"\x01" + bytes([0x22] * 32) + bytes.fromhex(topology) + bytes([0x11] * 32)
    assert len(head) == 97
    body = archive.instance_body(PIN_FIELDS, offset_milli=-5)
    instance = blake3(head + body, derive_key_context="quip family instance v1").hexdigest()
    assert instance == PIN_INSTANCE_DIGEST


def test_pack_spins_is_lsb_first_with_zero_padding() -> None:
    assert archive.pack_spins([1, -1, 1]) == "05"
    assert archive.pack_spins([1] * 8 + [-1, 1]) == "ff02"
    assert archive.pack_spins([-1] * 9) == "0000"
    assert archive.pack_spins([1] * 12) == "ff0f"


def test_i32_bounds_are_inclusive(tmp_path: Path) -> None:
    low = np.array([0, -(2**31), 0, 0], dtype=np.int64)
    assert read_model(_write(tmp_path / "low.npz", h_milli=low))["h"][1] == -(2**31)
    for value in (2**31, -(2**31) - 1):
        high = np.array([0, value, 0, 0], dtype=np.int64)
        with pytest.raises(ArchiveError, match="i32"):
            read_model(_write(tmp_path / "high.npz", h_milli=high))
