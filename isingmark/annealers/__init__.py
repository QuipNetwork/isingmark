# SPDX-License-Identifier: AGPL-3.0-or-later
"""Annealer adapter registry and ABC."""
from __future__ import annotations

from abc import ABC, abstractmethod

from isingmark.models import AnnealerSolution


class AnnealerAdapter(ABC):
    @property
    @abstractmethod
    def name(self) -> str: ...

    @abstractmethod
    def sample_ising(
        self,
        h: dict[int, float],
        J: dict[tuple[int, int], float],
        *,
        num_reads: int = 100,
        num_sweeps: int = 1000,
        **kwargs,
    ) -> AnnealerSolution: ...

    def setup(self) -> None: pass
    def teardown(self) -> None: pass


_REGISTRY: dict[str, AnnealerAdapter] = {}


def register(adapter: AnnealerAdapter) -> None:
    _REGISTRY[adapter.name] = adapter


def get_adapter(name: str) -> AnnealerAdapter:
    if name not in _REGISTRY:
        available = ", ".join(sorted(_REGISTRY)) or "(none)"
        raise KeyError(f"Unknown annealer {name!r}. Available: {available}")
    return _REGISTRY[name]


def list_adapters() -> list[str]:
    return sorted(_REGISTRY)


# Trigger registration of built-in adapters.
from isingmark.annealers import sa_dimod  # noqa: E402, F401
