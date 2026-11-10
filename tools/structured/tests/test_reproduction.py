"""Tests for the reproduction control writer, on a synthetic one-cell archive."""

from __future__ import annotations

import json
from pathlib import Path

import numpy as np
import pytest
from blake3 import blake3

from isingmark_structured.archive import ArchiveError
from isingmark_structured.reproduction import main, write_control

NONCE = "ab" * 32


def _model(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    np.savez(
        path,
        physical_nodes=np.array([4, 9, 30], dtype=np.int64),
        h_milli=np.array([500, -1000, 0], dtype=np.int64),
        edges=np.array([[0, 1], [1, 2]], dtype=np.int64),
        j_milli=np.array([-31, 250], dtype=np.int64),
    )


def _archive(tmp: Path, digest: str | None = None) -> tuple[Path, Path, Path]:
    root = tmp / "root"
    model = root / "cell/models/m.npz"
    _model(model)
    index = {
        "cells": [
            {
                "cell": "ice-charge",
                "rows": [
                    {
                        "nonce": NONCE,
                        "model": "cell/models/m.npz",
                        "model_blake3": digest or blake3(model.read_bytes()).hexdigest(),
                        "qpu80_pp": "-1523188",
                    }
                ],
            }
        ]
    }
    index_path = tmp / "index.json"
    index_path.write_text(json.dumps(index), encoding="utf-8")
    policies = tmp / "policies.json"
    policies.write_text(json.dumps({"schema_version": 2, "policies": []}), encoding="utf-8")
    return root, index_path, policies


def test_an_archived_model_becomes_a_fixed_instance(tmp_path: Path) -> None:
    root, index, policies = _archive(tmp_path)
    out = tmp_path / "control"
    record = write_control("ice-charge", index, root, policies, out)
    assert [row["nonce"] for row in record["rows"]] == [NONCE]
    body = json.loads((out / record["rows"][0]["file"]).read_text(encoding="utf-8"))
    assert body["family"] == "ising-fixed"
    assert body["nonce"] == NONCE
    assert body["physical"]["variables"] == ["n4", "n9", "n30"]
    assert body["physical"]["h"] == [0.5, -1.0, 0.0]
    assert body["physical"]["j"] == [-0.031, 0.25]
    assert body["physical"]["edges"] == [[0, 1], [1, 2]]
    # Strictly below (qpu80_pp + 1) milli means at or below qpu80_pp.
    assert body["objective"]["max_energy"] == -1523.187
    assert '"j":[-0.031,0.25]' in (out / record["rows"][0]["file"]).read_text(encoding="utf-8")
    manifest = json.loads((out / "manifest.json").read_text(encoding="utf-8"))
    assert manifest["instances"] == [record["rows"][0]["file"]]
    assert json.loads((out / "control.json").read_text(encoding="utf-8"))["rows"] == record["rows"]


def test_a_model_that_differs_from_its_digest_is_refused(tmp_path: Path) -> None:
    root, index, policies = _archive(tmp_path, digest="00" * 32)
    with pytest.raises(ArchiveError, match="differs from the index"):
        write_control("ice-charge", index, root, policies, tmp_path / "control")


def test_an_existing_directory_is_refused(tmp_path: Path) -> None:
    root, index, policies = _archive(tmp_path)
    assert (
        main(
            [
                "--cell",
                "ice-charge",
                "--policies",
                str(policies),
                "--out",
                str(tmp_path),
                "--index",
                str(index),
                "--research-root",
                str(root),
            ]
        )
        == 1
    )


def test_a_template_that_lists_instances_is_refused(tmp_path: Path) -> None:
    root, index, policies = _archive(tmp_path)
    policies.write_text(json.dumps({"instances": ["x"]}), encoding="utf-8")
    with pytest.raises(ArchiveError, match="must not list instances"):
        write_control("ice-charge", index, root, policies, tmp_path / "control")
