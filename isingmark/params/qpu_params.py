# SPDX-License-Identifier: AGPL-3.0-or-later
"""QPU parameter adaptor: maps hardness to D-Wave QPU annealing parameters."""
from __future__ import annotations

from typing import Any

from isingmark.params.adaptor import ParamsAdaptor


class QPUParamsAdaptor(ParamsAdaptor):
    """Maps problem hardness in [0, 1] to D-Wave QPU annealing parameters.

    Interpolates ``annealing_time`` logarithmically and ``num_reads`` linearly
    so that harder problems receive longer annealing pulses and more reads.

    Args:
        min_annealing_time: Minimum annealing time in microseconds (hardness=0).
        max_annealing_time: Maximum annealing time in microseconds (hardness=1).
        min_reads: Minimum number of reads (hardness=0).
        max_reads: Maximum number of reads (hardness=1).
    """

    def __init__(
        self,
        min_annealing_time: float = 1.0,
        max_annealing_time: float = 100.0,
        min_reads: int = 128,
        max_reads: int = 1024,
    ) -> None:
        self.min_annealing_time = min_annealing_time
        self.max_annealing_time = max_annealing_time
        self.min_reads = min_reads
        self.max_reads = max_reads

    def adapt(self, hardness: float, problem: Any = None, **kwargs: Any) -> dict[str, Any]:
        """Return num_reads and annealing_time for the given hardness.

        Annealing time is interpolated linearly between min and max.
        num_reads is interpolated linearly between min_reads and max_reads.

        Args:
            hardness: Problem difficulty in [0, 1]; clamped to that range.
            problem: Unused by this implementation.
            **kwargs: Ignored.

        Returns:
            Dict with keys ``num_reads`` and ``annealing_time``.
        """
        t = max(0.0, min(1.0, hardness))
        annealing_time = self.min_annealing_time + t * (
            self.max_annealing_time - self.min_annealing_time
        )
        num_reads = self.min_reads + int(t * (self.max_reads - self.min_reads))
        return {
            "num_reads": num_reads,
            "annealing_time": round(annealing_time, 3),
        }
