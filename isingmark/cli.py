# SPDX-License-Identifier: AGPL-3.0-or-later
"""CLI entry point for isingmark."""
from __future__ import annotations

import click


@click.group()
@click.version_option()
def main() -> None:
    """IsingMark: annealer-agnostic Ising model benchmarking."""


if __name__ == "__main__":
    main()
