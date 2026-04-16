# SPDX-License-Identifier: AGPL-3.0-or-later
"""Core data models for isingmark: problem definitions, solutions, and benchmark config."""

import dataclasses
from typing import Any


@dataclasses.dataclass(frozen=True)
class IsingProblem:
    """Immutable description of an Ising problem instance."""

    problem_id: str
    h: dict[int, float]
    J: dict[tuple[int, int], float]
    nodes: list[int]
    edges: list[tuple[int, int]]
    problem_type: str
    seed: int
    metadata: dict[str, Any]

    def to_json_dict(self) -> dict[str, Any]:
        """Serialize to a JSON-compatible dict.

        h keys become strings; J keys become "u,v" strings; edges become lists.
        """
        return {
            "problem_id": self.problem_id,
            "h": {str(k): v for k, v in self.h.items()},
            "J": {f"{u},{v}": w for (u, v), w in self.J.items()},
            "nodes": list(self.nodes),
            "edges": [list(e) for e in self.edges],
            "problem_type": self.problem_type,
            "seed": self.seed,
            "metadata": self.metadata,
        }

    @classmethod
    def from_json_dict(cls, d: dict[str, Any]) -> "IsingProblem":
        """Deserialize from a JSON-compatible dict produced by to_json_dict."""
        h = {int(k): float(v) for k, v in d["h"].items()}
        J = {}
        for key, w in d["J"].items():
            u_str, v_str = key.split(",", 1)
            J[(int(u_str), int(v_str))] = float(w)
        nodes = [int(n) for n in d["nodes"]]
        edges = [tuple(int(x) for x in e) for e in d["edges"]]
        return cls(
            problem_id=d["problem_id"],
            h=h,
            J=J,
            nodes=nodes,
            edges=edges,  # type: ignore[arg-type]
            problem_type=d["problem_type"],
            seed=int(d["seed"]),
            metadata=d["metadata"],
        )


@dataclasses.dataclass(frozen=True)
class AnnealerSolution:
    """Immutable record of an annealer's response to a single IsingProblem."""

    problem_id: str
    annealer_name: str
    params: dict[str, Any]
    wall_time_s: float
    best_energy: float
    mean_energy: float
    std_energy: float
    num_samples: int
    energies: list[float]
    diversity: float
    num_below_target: int
    satisfaction_rate: float
    hardware_timing: dict[str, Any]
    timestamp: float

    def to_json_dict(self) -> dict[str, Any]:
        """Serialize to a JSON-compatible dict via dataclasses.asdict."""
        return dataclasses.asdict(self)

    @classmethod
    def from_json_dict(cls, d: dict[str, Any]) -> "AnnealerSolution":
        """Deserialize from a JSON-compatible dict produced by to_json_dict."""
        return cls(
            problem_id=d["problem_id"],
            annealer_name=d["annealer_name"],
            params=d["params"],
            wall_time_s=float(d["wall_time_s"]),
            best_energy=float(d["best_energy"]),
            mean_energy=float(d["mean_energy"]),
            std_energy=float(d["std_energy"]),
            num_samples=int(d["num_samples"]),
            energies=[float(e) for e in d["energies"]],
            diversity=float(d["diversity"]),
            num_below_target=int(d["num_below_target"]),
            satisfaction_rate=float(d["satisfaction_rate"]),
            hardware_timing=d["hardware_timing"],
            timestamp=float(d["timestamp"]),
        )


@dataclasses.dataclass
class BenchmarkConfig:
    """Mutable configuration for a benchmarking run."""

    mode: str
    annealer_name: str
    problem_type: str
    problem_config: dict = dataclasses.field(default_factory=dict)
    hardness: float = 0.5
    duration_s: float = 600.0
    num_problems: int = 100
    seeds: list = dataclasses.field(default_factory=list)
    annealer_config: dict = dataclasses.field(default_factory=dict)
    param_overrides: dict = dataclasses.field(default_factory=dict)
    output_dir: str = "."
