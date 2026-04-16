# SPDX-License-Identifier: AGPL-3.0-or-later
"""MAX-CUT Ising problem generator.

Encodes the MAX-CUT problem as an Ising model: h=0 everywhere, J ∈ {-1, +1}
drawn uniformly at random via ChaCha8Rng.  Ground state unknown in general.
"""
from __future__ import annotations

import uuid
from typing import Any

from isingmark.chacha8 import ChaCha8Rng
from isingmark.models import IsingProblem
from isingmark.problems import register


class MaxCutGenerator:
    """Generates random MAX-CUT Ising problems.

    Sets h[node] = 0 for every node and draws J[u,v] ∈ {-1.0, +1.0}
    uniformly at random from ChaCha8Rng.  Maximising the number of
    satisfied couplings (J=-1 with opposing spins, or J=+1 with aligned
    spins) corresponds to the MAX-CUT objective.
    """

    @property
    def name(self) -> str:
        return "max_cut"

    def generate(
        self,
        seed: int,
        nodes: list[int],
        edges: list[tuple[int, int]],
        **kwargs: Any,
    ) -> IsingProblem:
        """Generate a random MAX-CUT Ising problem.

        Args:
            seed: u64 seed for ChaCha8Rng.
            nodes: List of node identifiers.
            edges: List of (u, v) edge tuples.
            **kwargs: Additional keyword arguments (ignored).

        Returns:
            IsingProblem with h=0 for all nodes and J ∈ {-1.0, +1.0} per edge.
        """
        rng = ChaCha8Rng.seed_from_u64(seed)

        h: dict[int, float] = {node: 0.0 for node in nodes}

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
            metadata={},
        )

    def known_ground_energy(self, problem: IsingProblem) -> float | None:
        """Return None — MAX-CUT ground state is NP-hard to compute in general."""
        return None


register(MaxCutGenerator())
