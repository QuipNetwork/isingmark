"""Compare regenerated models against the suite's saved models, by exact array equality.

A model matches when ``edges``, ``j_milli`` and ``h_milli`` equal the suite's
``work/cells/<family>/models/<nonce>.npz`` on the suite's own numbering,
before any fixture mapping. Extra arrays that both sides carry are compared
too and reported in their own column.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import numpy as np

from isingmark_structured import protocol
from isingmark_structured.families import FAMILIES
from isingmark_structured.topology import DEFAULT_SUITE_SPEC, SuiteSpec, load_suite_spec

REQUIRED = ("edges", "j_milli", "h_milli")


def verify_family(suite: SuiteSpec, suite_dir: Path, cell: str, count: int) -> tuple[int, int, int]:
    """``(matched, differing, extras differing)`` over the first ``count`` suite nonces."""
    family = FAMILIES[cell]
    models = suite_dir / "work" / "cells" / cell / "models"
    matched = differing = extras_differing = 0
    for nonce in protocol.make_nonces(count):
        model = family.generate(suite, nonce)
        with np.load(models / f"{nonce}.npz", allow_pickle=False) as saved:
            reference = dict(saved)
        reference.setdefault("h_milli", np.zeros(suite.n_spins, dtype=np.int64))
        if all(np.array_equal(model[key], reference[key]) for key in REQUIRED):
            matched += 1
        else:
            differing += 1
        shared = (set(model) & set(reference)) - set(REQUIRED)
        if any(not np.array_equal(model[key], reference[key]) for key in shared):
            extras_differing += 1
    return matched, differing, extras_differing


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite-dir", type=Path, required=True, help="the qpu-suite directory")
    parser.add_argument("--suite-spec", type=Path, default=DEFAULT_SUITE_SPEC)
    parser.add_argument(
        "--family", action="append", choices=sorted(FAMILIES), help="repeatable; default all"
    )
    parser.add_argument("--count", type=int, default=protocol.SUITE_MODELS)
    args = parser.parse_args(argv)
    cells = args.family or list(FAMILIES)
    suite = load_suite_spec(args.suite_spec)
    print(f"{'family':<20} {'matched':>8} {'differing':>10} {'extras differing':>17}")
    failed = False
    for cell in cells:
        matched, differing, extras = verify_family(suite, args.suite_dir, cell, args.count)
        failed |= differing > 0
        print(f"{cell:<20} {matched:>8} {differing:>10} {extras:>17}", flush=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
