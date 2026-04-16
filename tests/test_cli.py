# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for CLI commands."""
from __future__ import annotations

import json
from pathlib import Path

from click.testing import CliRunner

from isingmark.cli import main


def test_version():
    runner = CliRunner()
    result = runner.invoke(main, ["--version"])
    assert result.exit_code == 0
    assert "0.1.0" in result.output


def test_throughput_short_run():
    runner = CliRunner()
    with runner.isolated_filesystem() as tmpdir:
        result = runner.invoke(main, [
            "throughput",
            "--duration", "3s",
            "--hardness", "0.3",
            "--seed", "42",
            "--output-dir", tmpdir,
        ])
        assert result.exit_code == 0, result.output

        jsonl_files = list(Path(tmpdir).glob("results_*.jsonl"))
        assert len(jsonl_files) == 1

        summary_files = list(Path(tmpdir).glob("summary_*.json"))
        assert len(summary_files) == 1
        summary = json.loads(summary_files[0].read_text())
        assert summary["benchmark_mode"] == "throughput"
        assert summary["total_problems"] > 0
