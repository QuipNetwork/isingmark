"""Tests for the archive index writer, on a synthetic two-cell archive."""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from blake3 import blake3

from isingmark_structured import archive_index
from isingmark_structured.archive_index import build_index, main, render


def _archive(root: Path) -> dict[str, bytes]:
    """Two cells with two rows each. The second cell shares one model with the first."""
    models = {
        "a/models/m1.npz": b"model one",
        "a/models/m2.npz": b"model two",
        "b/models/m3.npz": b"model three",
    }
    for name, data in models.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)

    def row(nonce: str, name: str, best: int) -> dict[str, object]:
        return {
            "nonce": nonce,
            "model": str(root / name),
            "qpu80_best": best,
            "qpu80_greedy": best - 1,
            "qpu80_pp": best - 2,
            "pp_cpu_s": 0.5,
        }

    targets = {
        "cell-a": {
            "family": "x",
            "method": "pw",
            "budget_s": 0.25,
            "rows": [row("n1", "a/models/m1.npz", -10), row("n2", "a/models/m2.npz", -(2**40))],
        },
        "cell-b": {
            "family": "y",
            "method": "pt",
            "budget_s": 0.05,
            "rows": [row("n1", "a/models/m1.npz", -5), row("n3", "b/models/m3.npz", 7)],
        },
    }
    (root / "new-pow").mkdir()
    (root / "new-pow" / "targets-pp-100.json").write_text(json.dumps(targets), encoding="utf-8")
    return models


@pytest.fixture
def two_cells(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(archive_index, "CELLS", {"cell-a": "fam-a", "cell-b": "fam-b"})


def test_index_matches_archive(tmp_path: Path, two_cells: None) -> None:
    models = _archive(tmp_path)
    index = build_index(tmp_path)
    assert index["schema_revision"] == 1
    assert index["research_root"] == str(tmp_path)
    assert index["source"]["path"] == "new-pow/targets-pp-100.json"
    assert [c["cell"] for c in index["cells"]] == ["cell-a", "cell-b"]
    assert [c["family"] for c in index["cells"]] == ["fam-a", "fam-b"]
    first = index["cells"][0]
    assert first["method"] == "pw"
    assert first["budget_s"] == 0.25
    assert [r["nonce"] for r in first["rows"]] == ["n1", "n2"]
    row = first["rows"][0]
    assert row["model"] == "a/models/m1.npz"
    assert row["model_blake3"] == blake3(models["a/models/m1.npz"]).hexdigest()
    assert row["qpu80_best"] == "-10"
    assert row["qpu80_greedy"] == "-11"
    assert row["qpu80_pp"] == "-12"
    assert row["pp_cpu_s"] == 0.5
    assert first["rows"][1]["qpu80_best"] == str(-(2**40))


def test_shared_model_has_one_digest(tmp_path: Path, two_cells: None) -> None:
    _archive(tmp_path)
    index = build_index(tmp_path)
    shared = {c["cell"]: c["rows"][0]["model_blake3"] for c in index["cells"]}
    assert shared["cell-a"] == shared["cell-b"]


def test_model_outside_root_is_rejected(tmp_path: Path, two_cells: None) -> None:
    _archive(tmp_path)
    path = tmp_path / "new-pow" / "targets-pp-100.json"
    targets = json.loads(path.read_text(encoding="utf-8"))
    targets["cell-a"]["rows"][0]["model"] = "/elsewhere/m.npz"
    path.write_text(json.dumps(targets), encoding="utf-8")
    with pytest.raises(ValueError, match="not under"):
        build_index(tmp_path)


def test_missing_cell_is_rejected(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    _archive(tmp_path)
    monkeypatch.setattr(archive_index, "CELLS", {"cell-z": "fam"})
    with pytest.raises(ValueError, match="cell-z"):
        build_index(tmp_path)


def test_main_writes_rendered_index(
    tmp_path: Path, two_cells: None, capsys: pytest.CaptureFixture[str]
) -> None:
    _archive(tmp_path)
    out = tmp_path / "index.json"
    assert main(["--root", str(tmp_path), "--out", str(out)]) == 0
    text = out.read_text(encoding="utf-8")
    assert text == render(build_index(tmp_path))
    assert text.endswith("}\n")
    assert str(len(text.encode())) in capsys.readouterr().out
