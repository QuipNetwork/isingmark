# SPDX-License-Identifier: AGPL-3.0-or-later
"""Random Ising problem generator matching quip-protocol-2's generate_ising_model_from_nonce.

Produces byte-identical Ising problems to the Rust implementation using ChaCha8Rng.
h fields are generated first (one draw per node), then J fields (one draw per edge).
"""
from __future__ import annotations

import uuid

from isingmark.chacha8 import ChaCha8Rng
from isingmark.models import IsingProblem
from isingmark.problems import register

_DEFAULT_H_VALUES = [-1.0, 0.0, 1.0]


class RandomIsingGenerator:
    """Generates random Ising problems deterministically from a seed.

    Matches the quip-protocol-2 generate_ising_model_from_nonce algorithm:
    1. Seed ChaCha8Rng from the provided seed (u64).
    2. For each node in order: draw one u32, index into h_values.
    3. For each edge in order: draw one u32, assign -1.0 if even else 1.0.
    """

    @property
    def name(self) -> str:
        return "random_ising"

    def generate(
        self,
        seed: int,
        nodes: list[int],
        edges: list[tuple[int, int]],
        *,
        h_values: list[float] | None = None,
        **kwargs,
    ) -> IsingProblem:
        """Generate a random Ising problem for the given topology.

        Args:
            seed: u64 seed (replaces blockchain nonce in quip-protocol-2).
            nodes: List of node identifiers.
            edges: List of (u, v) edge tuples.
            h_values: Allowed local field values. Defaults to [-1.0, 0.0, 1.0].

        Returns:
            IsingProblem with h and J fields populated deterministically.
        """
        if h_values is None:
            h_values = _DEFAULT_H_VALUES
        n_h = len(h_values)

        rng = ChaCha8Rng.seed_from_u64(seed)

        h: dict[int, float] = {}
        for node in nodes:
            h[node] = h_values[rng.next_u32() % n_h]

        J: dict[tuple[int, int], float] = {}
        for edge in edges:
            J[tuple(edge)] = -1.0 if (rng.next_u32() & 1) == 0 else 1.0  # type: ignore[index]

        problem_id = str(uuid.uuid4())
        return IsingProblem(
            problem_id=problem_id,
            h=h,
            J=J,
            nodes=list(nodes),
            edges=[tuple(e) for e in edges],  # type: ignore[misc]
            problem_type=self.name,
            seed=seed,
            metadata={"h_values": h_values},
        )

    def known_ground_energy(self, problem: IsingProblem) -> float | None:
        """Return None — random Ising problems have no pre-computed ground state."""
        return None


# Register the singleton on import
register(RandomIsingGenerator())
