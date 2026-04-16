# SPDX-License-Identifier: AGPL-3.0-or-later
"""Problem generator registry and protocol."""
from __future__ import annotations

from typing import Protocol

from isingmark.models import IsingProblem


class ProblemGenerator(Protocol):
    @property
    def name(self) -> str: ...

    def generate(
        self,
        seed: int,
        nodes: list[int],
        edges: list[tuple[int, int]],
        **kwargs,
    ) -> IsingProblem: ...

    def known_ground_energy(
        self, problem: IsingProblem,
    ) -> float | None: ...


_REGISTRY: dict[str, ProblemGenerator] = {}


def register(gen: ProblemGenerator) -> None:
    _REGISTRY[gen.name] = gen


def get_generator(name: str) -> ProblemGenerator:
    if name not in _REGISTRY:
        available = ", ".join(sorted(_REGISTRY)) or "(none)"
        raise KeyError(f"Unknown problem type {name!r}. Available: {available}")
    return _REGISTRY[name]


def list_generators() -> list[str]:
    return sorted(_REGISTRY)


# Import generators to trigger their registration
from isingmark.problems import random_ising as _random_ising  # noqa: E402, F401
