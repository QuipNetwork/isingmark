"""Sparse nonce-defined local fields on the unchanged F11 ratio-eight models."""

from dataclasses import replace
import hashlib
import json
from pathlib import Path

import numpy as np
from quip_miner_dwave import regimes

from work import experiment, gpu_queue, ice, trimatch_power

FAMILIES = {"ice-fields-h500": 500, "ice-fields-h1000": 1000}
FIELD_DIVISOR = 16
FIELD_DOMAIN = b"ice-fields-v1"


def generate_model(nonce: str, field_milli: int) -> dict[str, np.ndarray]:
    """Bias shared spins after the F11 matching and gauge draws."""
    if type(field_milli) is not int or field_milli not in (500, 1000):
        raise ValueError("F24 field magnitudes are 500 and 1000 milli")
    model = ice.generate_model(nonce, 125)
    base, vertical, horizontal, _ = ice.topology()
    rng = np.random.default_rng(
        int.from_bytes(hashlib.sha256(bytes.fromhex(nonce)).digest(), "big")
    )
    rng.permutation(len(vertical))
    rng.permutation(len(horizontal))
    gauge = rng.choice(np.asarray([-1, 1], dtype=np.int8), size=base.n_spins)
    if not np.array_equal(gauge, model["gauge"]):
        raise ValueError("F11 gauge stream differs")
    memberships = np.bincount(model["tetrahedra"].ravel(), minlength=base.n_spins)
    eligible = np.flatnonzero(memberships == 2)
    count = base.n_spins // FIELD_DIVISOR
    if len(eligible) < count:
        raise ValueError("not enough shared spins for the fixed field count")
    selected = np.sort(rng.permutation(eligible)[:count]).astype(np.int64)
    field_cell = replace(
        base,
        name="ice-fields",
        alphabet_milli=[-field_milli, field_milli],
        draw_edges=base.draw_edges[: base.n_spins],
    )
    field_nonce = hashlib.sha256(bytes.fromhex(nonce) + FIELD_DOMAIN).hexdigest()
    drawn = regimes.draw_couplings(field_cell, field_nonce)
    model["h_milli"][selected] = drawn[selected] * gauge[selected]
    model["field_nodes"] = selected
    return model


def validate_baseline():
    """Verify the frozen construction, source files, and topology before output."""
    plan_path = experiment.ROOT / "work/f24-plan.json"
    runtime_path = experiment.ROOT / "work/f24-runtime.json"
    plan = json.loads(plan_path.read_text())
    runtime = json.loads(runtime_path.read_text())
    if plan["status"] != "FROZEN" or runtime["status"] != "FROZEN":
        raise ValueError("the F24 plan and runtime must be frozen")
    if plan["families"] != FAMILIES or plan["field_divisor"] != FIELD_DIVISOR:
        raise ValueError("the frozen F24 construction differs")
    if runtime["plan_sha256"] != hashlib.sha256(plan_path.read_bytes()).hexdigest():
        raise ValueError("the frozen F24 plan changed")
    for bindings in (plan["baseline_source_sha256"], runtime["source_sha256"]):
        for name, expected in bindings.items():
            if hashlib.sha256(Path(name).read_bytes()).hexdigest() != expected:
                raise ValueError(f"frozen F24 source changed: {name}")
    native, *_ = ice.topology()
    if native.n_spins != 4575 or len(native.draw_edges) != 41474:
        raise ValueError("F24 requires the live 4575-spin topology")
    return native


def main(family_dir: Path, field_milli: int) -> None:
    """Save all protocol models and refuse to replace different evidence."""
    if FAMILIES.get(family_dir.name) != field_milli:
        raise ValueError("family name differs from the field magnitude")
    native = validate_baseline()
    allowed = {(min(int(u), int(v)), max(int(u), int(v))) for u, v in native.draw_edges}
    nonces = regimes.make_nonces(24, 20260920)
    output = family_dir / "models"
    output.mkdir(parents=True, exist_ok=True)
    trimatch_power.save_json(family_dir / "nonces.json", nonces)
    rows = []
    for nonce in nonces:
        model = generate_model(nonce, field_milli)
        replay = generate_model(nonce, field_milli)
        for key in model:
            np.testing.assert_array_equal(model[key], replay[key])
        gpu_queue.validate_physical_model(model, allowed, native.n_spins)
        path = output / f"{nonce}.npz"
        trimatch_power.save_model(path, model)
        fields = model["h_milli"]
        incident = np.zeros(native.n_spins, dtype=np.int64)
        for endpoint in (0, 1):
            np.add.at(incident, model["edges"][:, endpoint], np.abs(model["j_milli"]))
        rows.append(
            {
                "nonce": nonce,
                "model_hash": experiment.model_hash(model),
                "archive_sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                "field_spins": int(np.count_nonzero(fields)),
                "positive_fields": int(np.count_nonzero(fields > 0)),
                "negative_fields": int(np.count_nonzero(fields < 0)),
                "field_rms_milli": float(np.sqrt(np.mean(fields.astype(float) ** 2))),
                "field_l1_over_coupling_l1": float(
                    np.abs(fields).sum() / np.abs(model["j_milli"]).sum()
                ),
                "strictly_field_dominated_spins": int(
                    np.count_nonzero(np.abs(fields) > incident)
                ),
                "tetrahedra": len(model["tetrahedra"]),
            }
        )
    sources = [
        Path(__file__),
        family_dir / "generate.py",
        Path(ice.__file__),
        Path(experiment.__file__),
        Path(trimatch_power.__file__),
        Path(gpu_queue.__file__),
        Path(regimes.__file__),
        ice.SPEC,
        experiment.ROOT / "work/f24-plan.json",
        experiment.ROOT / "work/f24-runtime.json",
    ]
    trimatch_power.save_json(
        family_dir / "manifest.json",
        {
            "family": family_dir.name,
            "field_milli": field_milli,
            "field_divisor": FIELD_DIVISOR,
            "spins": native.n_spins,
            "couplers": len(native.draw_edges),
            "numpy_version": np.__version__,
            "source_sha256": {
                str(path.resolve()): hashlib.sha256(path.read_bytes()).hexdigest()
                for path in sources
            },
            "models": rows,
        },
    )
    print({"family": family_dir.name, "models": len(rows)}, flush=True)
