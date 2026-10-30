"""Write a family's instances as an isingmark instance file and a manifest.

``<out>/<family>.jsonl`` holds a header line, then one line per nonce with
``h_milli`` and ``j_milli`` in the fixture's node and edge order.
``problems/<problem-family>/params/<cell>.json`` records the nonces, parameters,
provenance and, with ``--suite-dir``, the suite's per-model census.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from collections.abc import Iterable
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from isingmark_structured import protocol
from isingmark_structured.families import FAMILIES, Family
from isingmark_structured.topology import (
    DEFAULT_SUITE_SPEC,
    FixtureMap,
    SuiteSpec,
    load_fixture,
    load_suite_spec,
)

PACKAGE_DIR = Path(__file__).resolve().parent
REPO_ROOT = PACKAGE_DIR.parents[2]
DEFAULT_FIXTURE = REPO_ROOT / "fixtures" / "chain-h0.spec.json"
DEFAULT_OUT = REPO_ROOT / "out" / "structured"
DEFAULT_PROBLEMS = REPO_ROOT / "problems"
FORMAT = 1
MILLI_LIMIT = 1000
# Census lists longer than this (component sizes that count every isolated spin) stay in the suite.
CENSUS_LIST_LIMIT = 16


def package_sha256() -> str:
    """SHA-256 over every module of this package: relative path, a zero byte, then the bytes."""
    digest = hashlib.sha256()
    for path in sorted(PACKAGE_DIR.rglob("*.py")):
        digest.update(path.relative_to(PACKAGE_DIR).as_posix().encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
    return digest.hexdigest()


def write_instances(path: Path, header: dict[str, Any], instances: Iterable[dict[str, Any]]) -> int:
    """Write the header line and one line per instance. Returns the instance count."""
    count = 0
    with path.open("w", encoding="utf-8") as out:
        out.write(json.dumps({"kind": "header", **header}, separators=(",", ":")) + "\n")
        for instance in instances:
            for key in ("h_milli", "j_milli"):
                values = instance.get(key, [])
                if any(abs(v) > MILLI_LIMIT for v in values):
                    raise ValueError(f"{instance['nonce']}: a {key} value is outside [-1000, 1000]")
            out.write(json.dumps(instance, separators=(",", ":")) + "\n")
            count += 1
    return count


def read_instances(path: Path) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """The header and the instances of an instance file."""
    with path.open(encoding="utf-8") as lines:
        header = json.loads(next(lines))
        if header.get("kind") != "header" or header.get("format") != FORMAT:
            raise ValueError(f"{path} does not start with a format {FORMAT} header")
        return header, [json.loads(line) for line in lines if line.strip()]


def suite_record(suite_dir: Path, cell: str, family: Family, nonces: list[str]) -> dict[str, Any]:
    """The suite generator hash and the census of ``nonces`` from the suite's cell manifest."""
    manifest = json.loads((suite_dir / "work" / "cells" / cell / "manifest.json").read_text())
    generator = manifest.get("generator_sha256")
    if generator is None:
        sources = manifest.get("source_sha256", {})
        generator = next(
            (
                digest
                for name, digest in sources.items()
                if name.endswith(f"/work/{family.suite_source}")
            ),
            None,
        )
    census = {}
    for row in manifest.get("models") or manifest.get("cohort") or []:
        if row["nonce"] not in nonces:
            continue
        census[row["nonce"]] = {
            key: value
            for key, value in row.items()
            if key != "nonce"
            and not key.endswith("_s")
            and (not isinstance(value, list) or len(value) <= CENSUS_LIST_LIMIT)
        }
    return {"suite_generator_sha256": generator, "census": census}


def params_file(problems: Path, cell: str, problem_family: str | None) -> Path:
    """The ``problems/<family>/params/<cell>.json`` path for ``cell``.

    ``problem_family`` names the folder when given. Otherwise the folder is the one that
    already holds the cell; no match or several matches is an error.
    """
    if problem_family is not None:
        return problems / problem_family / "params" / f"{cell}.json"
    existing = sorted(problems.glob(f"*/params/{cell}.json"))
    if len(existing) == 1:
        return existing[0]
    if not existing:
        raise SystemExit(f"{cell}: no problems/*/params/{cell}.json yet; pass --problem-family")
    raise SystemExit(f"{cell}: several params files match ({existing}); pass --problem-family")


def build(
    cell: str,
    out: Path,
    params_path: Path,
    count: int,
    suite: SuiteSpec,
    fixture_path: Path,
    suite_dir: Path | None,
    references: dict[str, Any],
    update_params: bool = False,
) -> tuple[Path, Path]:
    family = FAMILIES[cell]
    fixture = load_fixture(fixture_path)
    mapping = FixtureMap(suite, fixture)
    if params_path.exists() and not update_params:
        nonces = json.loads(params_path.read_text(encoding="utf-8"))["nonces"]
    else:
        nonces = protocol.make_nonces(count)
    port = package_sha256()
    nonce_rule = f"make_nonces({count}, {protocol.NONCE_SEED})"
    header = {
        "format": FORMAT,
        "family": cell,
        "topology_hash": fixture.topology_hash(),
        "topology_fixture": fixture.name,
        "nodes": len(fixture.nodes),
        "edges": len(fixture.edges),
        "suite_topology_sha256": suite.sha256,
        "generator": "tools/structured",
        "generator_sha256": port,
        "params": family.params,
        "nonce_rule": nonce_rule,
        "created": datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ"),
    }

    def instances() -> Iterable[dict[str, Any]]:
        for nonce in nonces:
            model = family.generate(suite, nonce)
            h, j = mapping.map(model["edges"], model["j_milli"], model["h_milli"])
            instance: dict[str, Any] = {"nonce": nonce, "h_milli": h, "j_milli": j}
            if nonce in references:
                instance["reference"] = references[nonce]
            yield instance

    out.mkdir(parents=True, exist_ok=True)
    jsonl = out / f"{cell}.jsonl"
    write_instances(jsonl, header, instances())
    module_path = Path(str(family.module.__file__)).relative_to(PACKAGE_DIR.parent)
    manifest: dict[str, Any] = {
        "family": cell,
        "code": family.code,
        "params": family.params,
        "nonce_rule": nonce_rule,
        "nonces": nonces,
        "generator": "tools/structured",
        "port_module": module_path.as_posix(),
        "port_sha256": port,
        "suite_source": f"qpu-suite/work/{family.suite_source}",
        "exact": family.exact,
        "topology_fixture": fixture.name,
        "topology_hash": fixture.topology_hash(),
        "suite_topology_sha256": suite.sha256,
    }
    if suite_dir is not None:
        manifest.update(suite_record(suite_dir, cell, family, nonces))
    if references:
        manifest["references"] = {
            nonce: references[nonce] for nonce in nonces if nonce in references
        }
    if update_params or not params_path.exists():
        params_path.parent.mkdir(parents=True, exist_ok=True)
        params_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return jsonl, params_path


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--family",
        required=True,
        action="append",
        choices=sorted(FAMILIES),
        help="cell name; repeatable",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=DEFAULT_OUT,
        help="directory for <cell>.jsonl (default: out/structured/)",
    )
    parser.add_argument(
        "--problems",
        type=Path,
        default=DEFAULT_PROBLEMS,
        help="problems directory that holds <family>/params/<cell>.json (default: problems/)",
    )
    parser.add_argument(
        "--problem-family",
        help="problems/ folder for a cell that has no params file yet",
    )
    parser.add_argument("--count", type=int, default=protocol.SUITE_MODELS)
    parser.add_argument(
        "--fixture",
        type=Path,
        default=DEFAULT_FIXTURE,
        help="isingmark topology fixture (default: fixtures/chain-h0.spec.json)",
    )
    parser.add_argument(
        "--suite-spec", type=Path, default=DEFAULT_SUITE_SPEC, help="the suite's live topology spec"
    )
    parser.add_argument(
        "--suite-dir",
        type=Path,
        help="the qpu-suite directory; adds the suite generator hash and census",
    )
    parser.add_argument(
        "--update-params",
        action="store_true",
        help="replace an existing problems/<family>/params/<cell>.json (kept by default)",
    )
    parser.add_argument(
        "--references", type=Path, help="JSON object mapping a nonce to its reference energies"
    )
    args = parser.parse_args(argv)
    suite = load_suite_spec(args.suite_spec)
    references = json.loads(args.references.read_text()) if args.references else {}
    for cell in args.family:
        params_path = params_file(args.problems, cell, args.problem_family)
        jsonl, manifest = build(
            cell,
            args.out,
            params_path,
            args.count,
            suite,
            args.fixture,
            args.suite_dir,
            references,
            args.update_params,
        )
        print(f"{cell}: wrote {jsonl} and {manifest}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
