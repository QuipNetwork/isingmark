# SPDX-License-Identifier: AGPL-3.0-or-later
"""Parameter adaptation: map problem hardness to annealer parameters."""
from __future__ import annotations

from abc import ABC, abstractmethod
from typing import Any


class ParamsAdaptor(ABC):
    """Abstract base class for mapping hardness to annealer parameters."""

    @abstractmethod
    def adapt(self, hardness: float, problem: Any = None, **kwargs: Any) -> dict[str, Any]:
        """Map a hardness value in [0, 1] to annealer parameters.

        Args:
            hardness: Problem difficulty in [0, 1] (clamped internally).
            problem: Optional problem instance (subclasses may inspect it).
            **kwargs: Additional context for subclasses.

        Returns:
            Dict of annealer keyword arguments (e.g. num_sweeps, num_reads).
        """


class DefaultParamsAdaptor(ParamsAdaptor):
    """Linear interpolation from (min_sweeps, min_reads) to (max_sweeps, max_reads).

    Hardness is clamped to [0, 1] before interpolation so out-of-range values
    are treated identically to the boundary values.
    """

    def __init__(
        self,
        min_sweeps: int = 64,
        max_sweeps: int = 4096,
        min_reads: int = 64,
        max_reads: int = 512,
    ) -> None:
        self.min_sweeps = min_sweeps
        self.max_sweeps = max_sweeps
        self.min_reads = min_reads
        self.max_reads = max_reads

    def adapt(self, hardness: float, problem: Any = None, **kwargs: Any) -> dict[str, Any]:
        """Return num_sweeps and num_reads via linear interpolation on hardness.

        Args:
            hardness: Problem difficulty in [0, 1]; values outside the range are clamped.
            problem: Unused by this implementation.
            **kwargs: Ignored.

        Returns:
            Dict with keys ``num_sweeps`` and ``num_reads``.
        """
        t = max(0.0, min(1.0, hardness))
        num_sweeps = self.min_sweeps + int(t * (self.max_sweeps - self.min_sweeps))
        num_reads = self.min_reads + int(t * (self.max_reads - self.min_reads))
        return {"num_sweeps": num_sweeps, "num_reads": num_reads}
