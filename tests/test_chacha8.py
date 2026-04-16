# SPDX-License-Identifier: AGPL-3.0-or-later
"""Cross-language test vectors for ChaCha8Rng."""
from __future__ import annotations

import json
from pathlib import Path

from isingmark.chacha8 import ChaCha8Rng, seed_from_u64

VECTORS_PATH = Path(__file__).parent / "chacha8_test_vectors.json"
VECTORS = json.loads(VECTORS_PATH.read_text())


class TestSeedExpansion:
    def test_all_seed_vectors(self):
        for vec in VECTORS["seed_expansion"]:
            key = seed_from_u64(vec["seed"])
            assert key.hex() == vec["expected_key_hex"], (
                f"seed={vec['seed']}"
            )


class TestRngSequences:
    def test_all_sequence_vectors(self):
        for vec in VECTORS["rng_sequences"]:
            rng = ChaCha8Rng.seed_from_u64(vec["seed"])
            for i, expected in enumerate(vec["values"]):
                actual = rng.next_u32()
                assert actual == expected, (
                    f"seed={vec['seed']} index={i}: "
                    f"{actual} != {expected}"
                )
