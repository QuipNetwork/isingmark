# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for isingmark.models data classes."""

import json

import pytest

from isingmark.models import AnnealerSolution, BenchmarkConfig, IsingProblem


def make_ising_problem() -> IsingProblem:
    return IsingProblem(
        problem_id="test-001",
        h={0: 0.5, 1: -1.0},
        J={(0, 1): 0.25, (1, 2): -0.5},
        nodes=[0, 1, 2],
        edges=[(0, 1), (1, 2)],
        problem_type="random",
        seed=42,
        metadata={"hardness": 0.7},
    )


def make_annealer_solution() -> AnnealerSolution:
    return AnnealerSolution(
        problem_id="test-001",
        annealer_name="SimulatedAnnealing",
        params={"num_reads": 1000, "beta_range": [0.1, 10.0]},
        wall_time_s=1.23,
        best_energy=-2.5,
        mean_energy=-1.8,
        std_energy=0.4,
        num_samples=1000,
        energies=[-2.5, -2.0, -1.5],
        diversity=0.65,
        num_below_target=42,
        satisfaction_rate=0.042,
        hardware_timing={"qpu_access_time": 0.0},
        timestamp=1713225600.0,
    )


class TestIsingProblem:
    def test_ising_problem_frozen(self):
        prob = make_ising_problem()
        assert prob.problem_id == "test-001"
        assert prob.h == {0: 0.5, 1: -1.0}
        assert prob.J == {(0, 1): 0.25, (1, 2): -0.5}
        assert prob.nodes == [0, 1, 2]
        assert prob.edges == [(0, 1), (1, 2)]
        assert prob.problem_type == "random"
        assert prob.seed == 42
        assert prob.metadata == {"hardness": 0.7}

        with pytest.raises(Exception):
            prob.problem_id = "changed"  # type: ignore[misc]

    def test_ising_problem_to_json_roundtrip(self):
        original = make_ising_problem()
        json_str = json.dumps(original.to_json_dict())
        raw = json.loads(json_str)
        restored = IsingProblem.from_json_dict(raw)

        assert restored.problem_id == original.problem_id
        assert restored.h == original.h
        assert restored.J == original.J
        assert restored.nodes == original.nodes
        assert restored.edges == original.edges
        assert restored.problem_type == original.problem_type
        assert restored.seed == original.seed
        assert restored.metadata == original.metadata

    def test_ising_problem_json_key_format(self):
        prob = make_ising_problem()
        d = prob.to_json_dict()
        # h keys must be strings for JSON compatibility
        for k in d["h"]:
            assert isinstance(k, str)
        # J keys must be "u,v" strings
        for k in d["J"]:
            assert isinstance(k, str)
            assert "," in k
        # edges must be lists (not tuples)
        for edge in d["edges"]:
            assert isinstance(edge, list)


class TestAnnealerSolution:
    def test_annealer_solution_frozen(self):
        sol = make_annealer_solution()
        assert sol.problem_id == "test-001"
        assert sol.annealer_name == "SimulatedAnnealing"
        assert sol.best_energy == -2.5

        with pytest.raises(Exception):
            sol.problem_id = "changed"  # type: ignore[misc]

    def test_annealer_solution_to_json_roundtrip(self):
        original = make_annealer_solution()
        json_str = json.dumps(original.to_json_dict())
        raw = json.loads(json_str)
        restored = AnnealerSolution.from_json_dict(raw)

        assert restored.problem_id == original.problem_id
        assert restored.annealer_name == original.annealer_name
        assert restored.params == original.params
        assert restored.wall_time_s == original.wall_time_s
        assert restored.best_energy == original.best_energy
        assert restored.mean_energy == original.mean_energy
        assert restored.std_energy == original.std_energy
        assert restored.num_samples == original.num_samples
        assert restored.energies == original.energies
        assert restored.diversity == original.diversity
        assert restored.num_below_target == original.num_below_target
        assert restored.satisfaction_rate == original.satisfaction_rate
        assert restored.hardware_timing == original.hardware_timing
        assert restored.timestamp == original.timestamp


class TestBenchmarkConfig:
    def test_benchmark_config_defaults(self):
        cfg = BenchmarkConfig(mode="sweep", annealer_name="SA", problem_type="random")
        assert cfg.mode == "sweep"
        assert cfg.annealer_name == "SA"
        assert cfg.problem_type == "random"
        assert cfg.problem_config == {}
        assert cfg.hardness == 0.5
        assert cfg.duration_s == 600.0
        assert cfg.num_problems == 100
        assert cfg.seeds == []
        assert cfg.annealer_config == {}
        assert cfg.param_overrides == {}
        assert cfg.output_dir == "."

    def test_benchmark_config_mutable(self):
        cfg = BenchmarkConfig(mode="sweep", annealer_name="SA", problem_type="random")
        cfg.num_problems = 50
        assert cfg.num_problems == 50
