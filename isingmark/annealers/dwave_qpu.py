# SPDX-License-Identifier: AGPL-3.0-or-later
"""D-Wave QPU annealer adapter (optional dependency)."""
from __future__ import annotations

import time
from typing import Any

try:
    from dwave.system import AutoEmbeddingComposite, DWaveSampler

    _HAS_DWAVE = True
except ImportError:
    _HAS_DWAVE = False

from isingmark.annealers import AnnealerAdapter, register
from isingmark.models import AnnealerSolution
from isingmark.validator import SolutionValidator

import numpy as np


class DWaveQPUAdapter(AnnealerAdapter):
    """Adapter for a real D-Wave QPU via dwave-system.

    Uses AutoEmbeddingComposite for automatic minor-embedding.  Only
    available when ``dwave-system`` is installed and a valid solver token
    is configured in the D-Wave config file.

    Args:
        solver: Specific solver name to request, or None to use the default.
    """

    def __init__(self, solver: str | None = None) -> None:
        self._solver_name = solver
        self._composite: Any = None

    @property
    def name(self) -> str:
        return "dwave-qpu"

    def setup(self) -> None:
        """Initialise the DWaveSampler and AutoEmbeddingComposite.

        Raises:
            RuntimeError: If dwave-system is not installed.
        """
        if not _HAS_DWAVE:
            raise RuntimeError(
                "dwave-system is not installed. "
                "Install it with: pip install dwave-system"
            )
        kwargs: dict[str, Any] = {}
        if self._solver_name is not None:
            kwargs["solver"] = self._solver_name
        sampler = DWaveSampler(**kwargs)
        self._composite = AutoEmbeddingComposite(sampler)

    def sample_ising(
        self,
        h: dict[int, float],
        J: dict[tuple[int, int], float],
        *,
        num_reads: int = 100,
        num_sweeps: int = 1000,
        **kwargs: Any,
    ) -> AnnealerSolution:
        """Sample the Ising problem on the QPU.

        Args:
            h: Linear bias dict keyed by node id.
            J: Quadratic coupling dict keyed by (node_i, node_j) tuples.
            num_reads: Number of annealing shots.
            num_sweeps: Ignored for QPU; annealing_time (microseconds) is used
                instead if provided via kwargs.
            **kwargs: Optional ``annealing_time`` (float, microseconds),
                ``problem_id`` (str), ``seed`` (ignored for QPU).

        Returns:
            Populated AnnealerSolution instance.

        Raises:
            RuntimeError: If setup() has not been called.
        """
        if self._composite is None:
            raise RuntimeError("Call setup() before sample_ising().")

        annealing_time: float = float(kwargs.get("annealing_time", 20.0))
        problem_id: str = str(kwargs.get("problem_id", ""))

        sample_kwargs: dict[str, Any] = {
            "num_reads": num_reads,
            "annealing_time": annealing_time,
        }

        t_start = time.monotonic()
        sampleset = self._composite.sample_ising(h, J, **sample_kwargs)
        wall_time_s = time.monotonic() - t_start

        hardware_timing: dict[str, Any] = sampleset.info.get("timing", {})

        energies_arr = sampleset.record.energy
        samples_arr = sampleset.record.sample

        energies = [float(e) for e in energies_arr]
        best_idx = int(np.argmin(energies_arr))
        best_energy = float(energies_arr[best_idx])
        mean_energy = float(np.mean(energies_arr))
        std_energy = float(np.std(energies_arr))

        diversity_samples = [
            list(map(int, samples_arr[i])) for i in range(min(20, len(samples_arr)))
        ]
        diversity = SolutionValidator.compute_diversity(diversity_samples)

        nodes = list(sampleset.variables)
        best_spins = list(map(int, samples_arr[best_idx]))
        satisfaction_rate = SolutionValidator.coupling_satisfaction_rate(best_spins, J, nodes)

        params: dict[str, Any] = {
            "num_reads": num_reads,
            "annealing_time": annealing_time,
        }

        return AnnealerSolution(
            problem_id=problem_id,
            annealer_name=self.name,
            params=params,
            wall_time_s=wall_time_s,
            best_energy=best_energy,
            mean_energy=mean_energy,
            std_energy=std_energy,
            num_samples=len(energies),
            energies=energies,
            diversity=diversity,
            num_below_target=0,
            satisfaction_rate=satisfaction_rate,
            hardware_timing=hardware_timing,
            timestamp=time.time(),
        )


if _HAS_DWAVE:
    register(DWaveQPUAdapter())
