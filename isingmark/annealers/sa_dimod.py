# SPDX-License-Identifier: AGPL-3.0-or-later
"""SA-dimod annealer adapter wrapping dwave.samplers.SimulatedAnnealingSampler."""
from __future__ import annotations

import time
from typing import Any

import numpy as np
from dwave.samplers import SimulatedAnnealingSampler

from isingmark.annealers import AnnealerAdapter, register
from isingmark.models import AnnealerSolution
from isingmark.validator import SolutionValidator


class SADimodeAdapter(AnnealerAdapter):
    """Adapter for dwave-samplers SimulatedAnnealingSampler."""

    def __init__(self) -> None:
        self._sampler = SimulatedAnnealingSampler()

    @property
    def name(self) -> str:
        return "sa-dimod"

    def sample_ising(
        self,
        h: dict[int, float],
        J: dict[tuple[int, int], float],
        *,
        num_reads: int = 100,
        num_sweeps: int = 1000,
        **kwargs: Any,
    ) -> AnnealerSolution:
        """Sample the Ising problem and return an AnnealerSolution.

        Args:
            h: Linear bias dict keyed by node id.
            J: Quadratic coupling dict keyed by (node_i, node_j) tuples.
            num_reads: Number of annealing reads.
            num_sweeps: Number of sweeps per read.
            **kwargs: Optional seed (int) and problem_id (str).

        Returns:
            Populated AnnealerSolution instance.
        """
        seed = kwargs.get("seed")
        problem_id = kwargs.get("problem_id", "")

        sample_kwargs: dict[str, Any] = {
            "num_reads": num_reads,
            "num_sweeps": num_sweeps,
        }
        if seed is not None:
            sample_kwargs["seed"] = int(seed)

        t_start = time.monotonic()
        sampleset = self._sampler.sample_ising(h, J, **sample_kwargs)
        wall_time_s = time.monotonic() - t_start

        energies_arr = sampleset.record.energy
        samples_arr = sampleset.record.sample  # shape (num_reads, num_vars)

        energies = [float(e) for e in energies_arr]
        best_idx = int(np.argmin(energies_arr))
        best_energy = float(energies_arr[best_idx])
        mean_energy = float(np.mean(energies_arr))
        std_energy = float(np.std(energies_arr))

        # Use up to the first 20 samples for diversity computation.
        diversity_samples = [list(map(int, samples_arr[i])) for i in range(min(20, len(samples_arr)))]
        diversity = SolutionValidator.compute_diversity(diversity_samples)

        nodes = list(sampleset.variables)
        best_spins = list(map(int, samples_arr[best_idx]))
        satisfaction_rate = SolutionValidator.coupling_satisfaction_rate(best_spins, J, nodes)

        params: dict[str, Any] = {
            "num_reads": num_reads,
            "num_sweeps": num_sweeps,
        }
        if seed is not None:
            params["seed"] = int(seed)

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
            hardware_timing={},
            timestamp=time.time(),
        )


register(SADimodeAdapter())
