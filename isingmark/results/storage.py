# SPDX-License-Identifier: AGPL-3.0-or-later
"""JSONL and JSON storage utilities for AnnealerSolution results."""
from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from isingmark.models import AnnealerSolution


def append_solution_jsonl(path: str | Path, solution: AnnealerSolution) -> None:
    """Append one AnnealerSolution as a JSONL line to the given file.

    The file is created if it does not yet exist.

    Args:
        path: Destination file path.
        solution: The solution to serialize and append.
    """
    line = json.dumps(solution.to_json_dict(), separators=(",", ":"))
    with open(path, "a", encoding="utf-8") as fh:
        fh.write(line + "\n")


def read_solutions_jsonl(path: str | Path) -> list[AnnealerSolution]:
    """Read all AnnealerSolution records from a JSONL file.

    Args:
        path: Source file path.

    Returns:
        List of deserialized AnnealerSolution instances in file order.
    """
    solutions: list[AnnealerSolution] = []
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if line:
                solutions.append(AnnealerSolution.from_json_dict(json.loads(line)))
    return solutions


def write_summary_json(path: str | Path, summary: dict[str, Any]) -> None:
    """Write a summary dict as pretty-printed JSON.

    The file is created or overwritten.

    Args:
        path: Destination file path.
        summary: JSON-serializable dict to write.
    """
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(summary, fh, indent=2)
        fh.write("\n")
