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


FIELDS = {"node_ids": NODES, "edges": EDGES, "h": H, "j": J}
NONCE = "ab" * 32
# Distinct states with rising energy: LOW < MID < HIGH.
STATES = {
    "low": [1, 1, 1, 1],
    "mid": [-1, 1, -1, 1],
    "high": [1, -1, 1, -1],
}


def _e(name: str) -> int:
    state = np.array([STATES[name]], dtype=np.int8)
    return int(archive.energies_milli(FIELDS, state)[0])


def _states(*names: str) -> np.ndarray:
    return np.array([STATES[n] for n in names], dtype=np.int8).reshape(len(names), 4)


def _research(
    tmp_path: Path,
    *,
    pp_state: str | None = "low",
    pp_best: int | None = None,
    qpu: bool = True,
    gnat: tuple[str, ...] = ("high", "mid", "high"),
    loop: tuple[str, ...] | None = ("high", "high", "mid"),
    greedy_energy: int | None = None,
) -> tuple[Path, Path, str]:
    """A synthetic archive. The raw QPU best is read 1, so ``loop`` is read at index 1."""
    root = tmp_path / "root"
    (root / "models").mkdir(parents=True)
    model = _write(root / "models" / f"{NONCE}.npz")
    cell_dir = root / archive.MEASUREMENTS / "ice-tiles" / "toy"
    (cell_dir / "states").mkdir(parents=True)
    best = _e("low") if pp_best is None else pp_best
    (cell_dir / "results.jsonl").write_text(json.dumps({"nonce": NONCE, "pp_best": best}) + "\n")
    if pp_state is not None:
        np.save(cell_dir / "states" / f"{NONCE}.npy", _states(pp_state)[0])
    if qpu:
        reads = _states("high", "mid", "high")
        raw = archive.energies_milli(FIELDS, reads)
        qpu_dir = root / archive.QPU_ROOTS[1] / "toy" / "qpu-80"
        qpu_dir.mkdir(parents=True)
        np.savez(qpu_dir / f"{NONCE}.npz", spins=reads, energies=raw, mock=np.array(False))
    greedy_dir = root / archive.GREEDY / "toy"
    greedy_dir.mkdir(parents=True)
    arrays = {"gnat_c": _states(*gnat)}
    if loop is not None:
        arrays["loop_strict"] = _states(*loop)
    np.savez(greedy_dir / f"{NONCE}.states.npz", **arrays)
    optimizers = root / archive.OPTIMIZERS / "ice-tiles"
    optimizers.mkdir(parents=True)
    (optimizers / "b.py").write_text("b\n")
    (optimizers / "a.py").write_text("a\n")
    (optimizers / "lib.so").write_text("not python\n")
    if greedy_energy is None:
        names = [*gnat, *(loop[1:2] if loop else ())]
        greedy_energy = min(_e(n) for n in names)
    row = {
        "nonce": NONCE,
        "model": str(model),
        "qpu80_best": _e("mid"),
        "qpu80_greedy": greedy_energy,
    }
    targets = {
        "toy": {"family": "ice-tiles", "method": "anneal", "budget_s": 0.25, "rows": [row] * 2}
    }
    (root / "new-pow").mkdir(exist_ok=True)
    (root / archive.TARGETS).write_text(json.dumps(targets))
    return root, model, NONCE


def _build(root: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> dict:
    monkeypatch.setitem(archive.CELLS, "toy", ("toy-family", "toy-gen"))
    return archive.build_fixture(root, _repo(tmp_path), "toy")


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
    root, model, nonce = _research(tmp_path)
    doc = _build(root, tmp_path, monkeypatch)
    first = doc["archived_instances"][0]
    assert [i["name"] for i in doc["archived_instances"]] == ["toy-row0", "toy-row1"]
    assert first["archive_blake3"] == blake3(model.read_bytes()).hexdigest()
    assert first["archive_origin"] == f"models/{nonce}.npz"
    assert [w["name"] for w in first["witnesses"]] == ["qpu-best", "greedy", "pp-final"]
    assert first["postprocess_policy"]["sources"] == [
        {
            "path": f"{archive.OPTIMIZERS}/ice-tiles/{name}.py",
            "blake3": blake3(f"{name}\n".encode()).hexdigest(),
        }
        for name in ("a", "b")
    ]
    assert (first["n_nodes"], first["n_edges"]) == (4, 3)
    assert "edges" not in first
    assert len(first["model_digest"]) == 64
    assert doc["sources"][1]["path"] == "gen.py"
    assert "archived observation" in doc["about"][1]
    assert render(doc).endswith("}\n")


def test_witness_origins_name_the_archive_files(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root, _, nonce = _research(tmp_path)
    qpu, greedy, pp = _build(root, tmp_path, monkeypatch)["archived_instances"][0]["witnesses"]
    qpu_file = root / archive.QPU_ROOTS[1] / "toy" / "qpu-80" / f"{nonce}.npz"
    assert qpu["origin"] == {
        "path": f"{archive.QPU_ROOTS[1]}/toy/qpu-80/{nonce}.npz",
        "blake3": blake3(qpu_file.read_bytes()).hexdigest(),
        "array": "spins",
        "index": 1,
    }
    assert qpu["claimed_energy_milli"] == str(_e("mid"))
    assert qpu["spins_packed"] == archive.pack_spins(STATES["mid"])
    assert greedy["origin"]["path"] == f"{archive.GREEDY}/toy/{nonce}.states.npz"
    assert pp["origin"]["array"] is None
    assert pp["origin"]["index"] is None
    assert pp["origin"]["path"].endswith(f"toy/states/{nonce}.npy")
    assert pp["claimed_energy_milli"] == str(_e("low"))


def test_greedy_takes_loop_state_of_the_best_raw_read_on_a_tie(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # gnat_c best is "mid" at read 1, loop_strict[1] is also "mid": a tie goes to loop_strict.
    root, _, _ = _research(tmp_path, gnat=("high", "mid", "high"), loop=("high", "mid", "high"))
    greedy = _build(root, tmp_path, monkeypatch)["archived_instances"][0]["witnesses"][1]
    assert (greedy["origin"]["array"], greedy["origin"]["index"]) == ("loop_strict", 1)


def test_greedy_takes_loop_state_when_it_is_lower(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root, _, _ = _research(tmp_path, gnat=("high", "mid", "high"), loop=("high", "low", "high"))
    greedy = _build(root, tmp_path, monkeypatch)["archived_instances"][0]["witnesses"][1]
    assert (greedy["origin"]["array"], greedy["origin"]["index"]) == ("loop_strict", 1)
    assert greedy["claimed_energy_milli"] == str(_e("low"))


def test_greedy_takes_gnat_c_when_it_wins(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    # gnat_c holds "low" at read 2, away from the raw best read 1. loop_strict[1] is only "mid".
    root, _, _ = _research(tmp_path, gnat=("high", "mid", "low"), loop=("high", "mid", "high"))
    greedy = _build(root, tmp_path, monkeypatch)["archived_instances"][0]["witnesses"][1]
    assert (greedy["origin"]["array"], greedy["origin"]["index"]) == ("gnat_c", 2)
    assert greedy["claimed_energy_milli"] == str(_e("low"))


def test_greedy_without_loop_strict_uses_gnat_c(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root, _, _ = _research(tmp_path, loop=None)
    greedy = _build(root, tmp_path, monkeypatch)["archived_instances"][0]["witnesses"][1]
    assert (greedy["origin"]["array"], greedy["origin"]["index"]) == ("gnat_c", 1)


def test_greedy_with_an_empty_loop_strict_uses_gnat_c(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root, _, _ = _research(tmp_path, loop=())
    greedy = _build(root, tmp_path, monkeypatch)["archived_instances"][0]["witnesses"][1]
    assert greedy["origin"]["array"] == "gnat_c"


def test_greedy_energy_mismatch_stops_the_export(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root, _, _ = _research(tmp_path, greedy_energy=_e("low"))
    with pytest.raises(ArchiveError, match="greedy: state energy"):
        _build(root, tmp_path, monkeypatch)


def test_qpu_energy_mismatch_stops_the_export(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root, _, _ = _research(tmp_path)
    targets = json.loads((root / archive.TARGETS).read_text())
    targets["toy"]["rows"][0]["qpu80_best"] += 1
    (root / archive.TARGETS).write_text(json.dumps(targets))
    with pytest.raises(ArchiveError, match="qpu-best: state energy"):
        _build(root, tmp_path, monkeypatch)


def test_mock_reads_are_rejected(tmp_path: Path) -> None:
    root, _, nonce = _research(tmp_path)
    path = root / archive.QPU_ROOTS[1] / "toy" / "qpu-80" / f"{nonce}.npz"
    reads = _states("high", "mid", "high")
    np.savez(path, spins=reads, energies=archive.energies_milli(FIELDS, reads), mock=np.array(True))
    with pytest.raises(ArchiveError, match="mock"):
        archive.qpu_read(root, "toy", nonce, 4)


def test_missing_archive_files_are_recorded_with_the_searched_paths(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root, _, nonce = _research(tmp_path, qpu=False, pp_state=None, loop=None)
    qpu, greedy, pp = _build(root, tmp_path, monkeypatch)["archived_instances"][0]["witnesses"]
    for base in archive.QPU_ROOTS:
        assert f"{base}/toy/qpu-80/{nonce}.npz" in qpu["missing"]
    assert set(qpu) == {"name", "missing"}
    assert "origin" in greedy
    assert (
        pp["missing"]
        == f"state not stored, searched {archive.MEASUREMENTS}/ice-tiles/toy/states/{nonce}.npy"
    )
    assert ";" not in qpu["missing"] + pp["missing"]


def test_loop_states_without_qpu_reads_stop_the_export(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root, _, _ = _research(tmp_path, qpu=False)
    with pytest.raises(ArchiveError, match="greedy rule cannot be applied"):
        _build(root, tmp_path, monkeypatch)


def test_absent_greedy_file_is_recorded_without_an_origin(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root, _, nonce = _research(tmp_path)
    (root / archive.GREEDY / "toy" / f"{nonce}.states.npz").unlink()
    greedy = _build(root, tmp_path, monkeypatch)["archived_instances"][0]["witnesses"][1]
    assert greedy == {
        "name": "greedy",
        "missing": f"states not stored, searched {archive.GREEDY}/toy/{nonce}.states.npz",
    }


@pytest.mark.parametrize("key", ["gnat_c", "loop_strict"])
def test_one_dimensional_greedy_states_are_rejected(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, key: str
) -> None:
    root, _, nonce = _research(tmp_path)
    path = root / archive.GREEDY / "toy" / f"{nonce}.states.npz"
    arrays = {"gnat_c": _states("low", "mid", "high"), "loop_strict": _states("low", "mid", "high")}
    arrays[key] = _states("low")[0]
    np.savez(path, **arrays)
    with pytest.raises(ArchiveError, match="2 dimensions"):
        _build(root, tmp_path, monkeypatch)


def test_one_dimensional_qpu_spins_are_rejected(tmp_path: Path) -> None:
    root, _, nonce = _research(tmp_path)
    path = root / archive.QPU_ROOTS[1] / "toy" / "qpu-80" / f"{nonce}.npz"
    np.savez(path, spins=_states("low")[0], energies=np.zeros(1), mock=np.array(False))
    with pytest.raises(ArchiveError, match="2 dimensions"):
        archive.qpu_read(root, "toy", nonce, 4)


def test_qpu_ties_take_the_first_read(tmp_path: Path) -> None:
    root, _, nonce = _research(tmp_path)
    path = root / archive.QPU_ROOTS[1] / "toy" / "qpu-80" / f"{nonce}.npz"
    reads = _states("high", "mid", "mid")
    energies = archive.energies_milli(FIELDS, reads)
    np.savez(path, spins=reads, energies=energies, mock=np.array(False))
    assert archive.qpu_read(root, "toy", nonce, 4)[2] == 1


def test_first_qpu_root_with_the_file_wins(tmp_path: Path) -> None:
    root, _, nonce = _research(tmp_path)
    first = root / archive.QPU_ROOTS[0] / "toy" / "qpu-80"
    first.mkdir(parents=True)
    reads = _states("high", "mid", "low")
    np.savez(
        first / f"{nonce}.npz",
        spins=reads,
        energies=archive.energies_milli(FIELDS, reads),
        mock=np.array(False),
    )
    path, _, best = archive.qpu_read(root, "toy", nonce, 4)
    assert path == first / f"{nonce}.npz"
    assert best == 2


def test_bad_qpu_states_are_rejected(tmp_path: Path) -> None:
    root, _, nonce = _research(tmp_path)
    path = root / archive.QPU_ROOTS[1] / "toy" / "qpu-80" / f"{nonce}.npz"
    np.savez(
        path, spins=np.zeros((3, 4), dtype=np.int8), energies=np.zeros(3), mock=np.array(False)
    )
    with pytest.raises(ArchiveError, match="not 1 or -1"):
        archive.qpu_read(root, "toy", nonce, 4)
    np.savez(
        path, spins=_states("low").astype(np.int64), energies=np.zeros(1), mock=np.array(False)
    )
    with pytest.raises(ArchiveError, match="int8"):
        archive.qpu_read(root, "toy", nonce, 4)


def test_object_array_state_is_rejected(tmp_path: Path) -> None:
    root, _, nonce = _research(tmp_path)
    path = root / archive.GREEDY / "toy" / f"{nonce}.states.npz"
    ragged = np.empty(2, dtype=object)
    ragged[:] = [[1, -1], [1]]
    np.savez(path, gnat_c=ragged)
    with pytest.raises(ArchiveError, match="gnat_c"):
        archive.load_array(path, "gnat_c")


def test_policy_without_sources_is_rejected(tmp_path: Path) -> None:
    with pytest.raises(ArchiveError, match="no optimizer sources"):
        archive.policy_sources(tmp_path, "ice-tiles")


def test_missing_pp_state_is_recorded_with_the_searched_path(tmp_path: Path) -> None:
    root, _, nonce = _research(tmp_path, pp_state=None)
    entry = archive.pp_witness(root, "ice-tiles", "toy", nonce, FIELDS)
    assert entry["name"] == "pp-final"
    assert f"states/{nonce}.npy" in entry["missing"]


def test_pp_energy_mismatch_is_rejected(tmp_path: Path) -> None:
    root, _, nonce = _research(tmp_path, pp_best=-1)
    with pytest.raises(ArchiveError, match="pp-final: state energy"):
        archive.pp_witness(root, "ice-tiles", "toy", nonce, FIELDS)


def test_bad_pp_state_is_rejected(tmp_path: Path) -> None:
    root, _, nonce = _research(tmp_path)
    path = root / archive.MEASUREMENTS / "ice-tiles" / "toy" / "states" / f"{nonce}.npy"
    np.save(path, np.array([1, -1, 0, -1], dtype=np.int8))
    with pytest.raises(ArchiveError, match="not 1 or -1"):
        archive.pp_witness(root, "ice-tiles", "toy", nonce, FIELDS)


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
