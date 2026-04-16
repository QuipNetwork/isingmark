# SPDX-License-Identifier: AGPL-3.0-or-later
"""Solution validator for Ising benchmark results."""

from __future__ import annotations

from typing import TYPE_CHECKING

import numpy as np

if TYPE_CHECKING:
    from isingmark.models import AnnealerSolution, IsingProblem


class SolutionValidator:
    """Static utility methods for validating and scoring Ising solutions."""

    @staticmethod
    def compute_energy(
        spins: list[int],
        h: dict[int, float],
        J: dict[tuple[int, int], float],
        nodes: list[int],
    ) -> float:
        """Compute Ising energy E = sum(h_i*s_i) + sum(J_ij*s_i*s_j).

        Args:
            spins: Spin values (+1/-1), one per node in `nodes` order.
            h: Linear bias dict keyed by node id.
            J: Quadratic coupling dict keyed by (node_i, node_j) tuples.
            nodes: List of node ids defining the variable ordering for `spins`.

        Returns:
            Scalar Ising energy.
        """
        node_pos: dict[int, int] = {int(nid): pos for pos, nid in enumerate(nodes)}

        energy = 0.0
        for nid, bias in h.items():
            energy += bias * spins[node_pos[int(nid)]]

        for (u, v), coupling in J.items():
            energy += coupling * spins[node_pos[int(u)]] * spins[node_pos[int(v)]]

        return energy

    @staticmethod
    def validate_format(spins: list[int], num_nodes: int) -> list[str]:
        """Check that spins has the correct length and all values in {-1, +1}.

        Args:
            spins: Candidate spin assignment.
            num_nodes: Expected number of spins.

        Returns:
            List of error strings; empty list if valid.
        """
        errors: list[str] = []

        if len(spins) != num_nodes:
            errors.append(
                f"Length mismatch: got {len(spins)} spins, expected {num_nodes}."
            )

        invalid = [s for s in spins if s not in (-1, 1)]
        if invalid:
            errors.append(
                f"Invalid spin values (must be -1 or +1): {invalid}."
            )

        return errors

    @staticmethod
    def compute_diversity(solutions: list[list[int]]) -> float:
        """Average normalized symmetric Hamming distance across all solution pairs.

        Symmetric means we take min(hamming(a, b), hamming(a, -b)) / n so that
        global spin-flip equivalents are treated as identical.

        Args:
            solutions: List of spin assignments, each of length n.

        Returns:
            Average pairwise symmetric Hamming distance in [0, 1].
            Returns 0.0 if fewer than 2 solutions are provided.
        """
        if len(solutions) < 2:
            return 0.0

        arr = np.array(solutions, dtype=np.int8)  # shape (m, n)
        m, n = arr.shape
        if n == 0:
            return 0.0

        total = 0.0
        count = 0
        for i in range(m):
            for j in range(i + 1, m):
                diff = int(np.sum(arr[i] != arr[j]))
                diff_flipped = int(np.sum(arr[i] != -arr[j]))
                total += min(diff, diff_flipped) / n
                count += 1

        return total / count if count > 0 else 0.0

    @staticmethod
    def coupling_satisfaction_rate(
        spins: list[int],
        J: dict[tuple[int, int], float],
        nodes: list[int],
    ) -> float:
        """Fraction of couplings where J_ij * s_i * s_j < 0 (negative contribution).

        A coupling is "satisfied" when it contributes negative energy.

        Args:
            spins: Spin values in `nodes` order.
            J: Coupling dict keyed by (node_i, node_j).
            nodes: Node ids defining spin ordering.

        Returns:
            Fraction of satisfied couplings in [0, 1]. Returns 0.0 if J is empty.
        """
        if not J:
            return 0.0

        node_pos: dict[int, int] = {int(nid): pos for pos, nid in enumerate(nodes)}
        satisfied = sum(
            1
            for (u, v), coupling in J.items()
            if coupling * spins[node_pos[int(u)]] * spins[node_pos[int(v)]] < 0
        )
        return satisfied / len(J)

    @staticmethod
    def validate_solution(
        problem: "IsingProblem",
        result: "AnnealerSolution",
        spins_list: list[list[int]],
        energy_target: float | None = None,
    ) -> dict:
        """Full validation report for an annealer result against a problem.

        Args:
            problem: The IsingProblem being solved.
            result: The AnnealerSolution record (used for metadata).
            spins_list: All spin assignments returned by the annealer.
            energy_target: Optional energy threshold for counting solutions below it.

        Returns:
            Dict with keys:
                valid (bool), errors (list[str]), best_energy (float),
                diversity (float), satisfaction_rate (float),
                and num_below_target (int) when energy_target is provided.
        """
        n = len(problem.nodes)
        all_errors: list[str] = []

        energies: list[float] = []
        for spins in spins_list:
            fmt_errors = SolutionValidator.validate_format(spins, n)
            all_errors.extend(fmt_errors)
            if not fmt_errors:
                e = SolutionValidator.compute_energy(spins, problem.h, problem.J, problem.nodes)
                energies.append(e)

        best_energy = min(energies) if energies else float("inf")

        best_spins: list[int] = []
        if energies:
            best_idx = int(np.argmin(energies))
            best_spins = spins_list[best_idx]

        diversity = SolutionValidator.compute_diversity(spins_list)

        satisfaction = (
            SolutionValidator.coupling_satisfaction_rate(best_spins, problem.J, problem.nodes)
            if best_spins
            else 0.0
        )

        report: dict = {
            "valid": len(all_errors) == 0,
            "errors": all_errors,
            "best_energy": best_energy,
            "diversity": diversity,
            "satisfaction_rate": satisfaction,
        }

        if energy_target is not None:
            report["num_below_target"] = sum(1 for e in energies if e < energy_target)

        return report
