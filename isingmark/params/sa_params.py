# SPDX-License-Identifier: AGPL-3.0-or-later
"""SA-tuned parameter adaptor: sweeps scale as sqrt(hardness)."""
from __future__ import annotations

import math
from typing import Any

from isingmark.params.adaptor import ParamsAdaptor


class SAParamsAdaptor(ParamsAdaptor):
    """Maps problem hardness to SA parameters with sqrt-scaled sweeps.

    Unlike DefaultParamsAdaptor (linear interpolation), sweep counts here
    scale as ``sqrt(hardness)`` so that mid-range problems receive
    proportionally more sweeps than a linear schedule would give them.
    num_reads is still interpolated linearly.

    Args:
        min_sweeps: Minimum number of sweeps (hardness=0).
        max_sweeps: Maximum number of sweeps (hardness=1).
        min_reads: Minimum number of reads (hardness=0).
        max_reads: Maximum number of reads (hardness=1).
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
        """Return num_sweeps and num_reads for the given hardness.

        num_sweeps is computed as ``min_sweeps + sqrt(t) * (max_sweeps - min_sweeps)``
        where ``t = clamp(hardness, 0, 1)``.
        num_reads is linearly interpolated.

        Args:
            hardness: Problem difficulty in [0, 1]; clamped to that range.
            problem: Unused by this implementation.
            **kwargs: Ignored.

        Returns:
            Dict with keys ``num_sweeps`` and ``num_reads``.
        """
        t = max(0.0, min(1.0, hardness))
        sqrt_t = math.sqrt(t)
        num_sweeps = self.min_sweeps + int(sqrt_t * (self.max_sweeps - self.min_sweeps))
        num_reads = self.min_reads + int(t * (self.max_reads - self.min_reads))
        return {"num_sweeps": num_sweeps, "num_reads": num_reads}
