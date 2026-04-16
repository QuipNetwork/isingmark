# CLAUDE.md

## Overview

IsingMark: standalone annealer-agnostic Ising model benchmarking tool.
Extracted from quip-protocol-2. Compares any Ising solver via reproducible benchmarks.

## Commands

```bash
# Setup
python3 -m venv .venv && source .venv/bin/activate
pip install -e ".[dev]"

# Run tests
python -m pytest tests/ -v

# CLI
isingmark --help
isingmark throughput --duration 10m --hardness 0.5
isingmark sweep --param-grid '{"num_reads": [64,128], "num_sweeps": [100,500]}'
isingmark ttt --energy-targets "-5,-10,-15" --num-trials 20
isingmark compare --file-a results_a.jsonl --file-b results_b.jsonl --plot
isingmark scaling --sizes "10,20,50,100"
isingmark reproducibility --num-problems 20
```

## Architecture

Three extension points:
- **ProblemGenerator** (Protocol) — `isingmark/problems/`
- **AnnealerAdapter** (ABC) — `isingmark/annealers/`
- **ParamsAdaptor** (ABC) — `isingmark/params/`

Multiprocessing with spawn context. JSONL + summary JSON output.

## License

AGPL-3.0-or-later
