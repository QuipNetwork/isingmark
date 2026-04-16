# SPDX-License-Identifier: AGPL-3.0-or-later
"""CUDA simulated annealing adapter stub (not yet implemented)."""
from __future__ import annotations

from typing import Any

from isingmark.annealers import AnnealerAdapter, register
from isingmark.models import AnnealerSolution

_HAS_CUDA_SA = False


class CUDASAAdapter(AnnealerAdapter):
    """Stub adapter for a CUDA-accelerated SA sampler.

    This adapter is not yet implemented.  It exists to reserve the
    ``cuda-sa`` name in the registry when the CUDA SA library becomes
    available.

    Args:
        device: CUDA device string, e.g. ``"cuda:0"``.
    """

    def __init__(self, device: str = "cuda:0") -> None:
        self._device = device

    @property
    def name(self) -> str:
        return "cuda-sa"

    def sample_ising(
        self,
        h: dict[int, float],
        J: dict[tuple[int, int], float],
        *,
        num_reads: int = 100,
        num_sweeps: int = 1000,
        **kwargs: Any,
    ) -> AnnealerSolution:
        """Not implemented — raises NotImplementedError.

        Args:
            h: Linear bias dict keyed by node id.
            J: Quadratic coupling dict keyed by (node_i, node_j) tuples.
            num_reads: Number of annealing reads.
            num_sweeps: Number of sweeps per read.
            **kwargs: Additional keyword arguments (ignored).

        Raises:
            NotImplementedError: Always.
        """
        raise NotImplementedError(
            "CUDA SA adapter is not yet implemented. "
            "Install the cuda-sa package and set _HAS_CUDA_SA = True to enable."
        )


if _HAS_CUDA_SA:
    register(CUDASAAdapter())
