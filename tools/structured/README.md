# Structured instance generators

The `isingmark-structured` package generates the structured Ising families of the QPU suite and writes them as isingmark instance files.
Each family is a deterministic function of a 32-byte nonce and the live topology specification of the chip.
isingmark replays the files with `--instances`.

## Families

| Cell | Family | Tier | Parameters |
| --- | --- | --- | --- |
| `ice-glass-r8` | F11 | 1 | `background_milli` 125 |
| `ice-glass-r5` | F11 | 1 | `background_milli` 200 |
| `ice-twisted` | F12 | 2 | none |
| `tri-match-r4` | F1 | 1 | `ratio` 4 (background 250 milli) |
| `tri-match-r6` | F1 | 1 | `ratio` 6 (background 167 milli) |
| `strong-net-d1.5` | F2 | 1 | `density` 1.5 |
| `strong-net-d2` | F2 | 1 | `density` 2 |
| `strong-net-d3` | F2 | 1 | `density` 3 |
| `diamond-3d` | F15 | 1 | none |
| `cubic-no-dimer` | F16 | 1 | none |
| `dimer-power-mixed` | F6 | 1 | none |
| `dcl-k44-l5` | F5 | 1 | `cell_milli` 200 |
| `dcl-k44-l7` | F5 | 1 | `cell_milli` 143 |
| `fcl-planted-a0.2` | F4 | 1 | `density_tenths` 2 |
| `fcl-planted-a0.3` | F4 | 1 | `density_tenths` 3 |

The module docstring of each file under `isingmark_structured/families/` describes the construction.

## Install

The package needs Python 3.11 or later, NumPy, and SciPy.
It does not need the D-Wave Ocean packages or the miner's Python bindings.
It carries its own ports of the protocol draw and of the Zephyr coordinate helpers.

```bash
cd tools/structured
python -m venv .venv
.venv/bin/pip install -e '.[test]'
```

## Generate

The generator reads the live topology specification of the suite.
The default path is `fixtures/suite-live-2026-09-21.spec.json` in this repository.
Give another copy with `--suite-spec`.
The specification has 4,575 qubits and 41,474 couplers.

```bash
isingmark-structured --family ice-glass-r8
isingmark-structured --family tri-match-r4 --family tri-match-r6 --count 24
```

Each run writes two files for each family:

- `corpora/<cell>.jsonl` (set the directory with `--out`): a header line, then one instance per nonce.
- `problems/<family>/params/<cell>.json`: the nonces and parameters of the run.
  A cell with no params file yet needs `--problem-family <family>`.
  It also holds the package hash and the exactness flag.

Add `--suite-dir <qpu-suite>` to copy the suite generator hash and the per-model census into the manifest.
Add `--references <file>` with a JSON object from nonce to reference energies to attach a `reference` object to each instance.

The nonces come from `make_nonces(count, 20260920)`.
The NumPy generator `default_rng(20260920)` draws 32 bytes for each nonce.
The first 24 nonces are the suite cohort for every count of 24 or more.

### Topology mapping

The suite models use the live working graph in dense numbering.
The isingmark fixture `chain-h0` is the same chip with 4,577 qubits and 41,515 couplers.
The generator maps every model to the fixture by qubit id.
It writes `j_milli` in the edge order of the fixture and `h_milli` in the node order of the fixture.
The two fixture qubits and the 41 fixture couplers that the suite specification lacks get zero.
The generator stops with an error when the fixture lacks a qubit or a coupler of the suite specification.

The header records `topology_hash`, the isingmark topology hash of the fixture.
For `chain-h0` this value is `e66d3dfa3c9c6afb15efe29891cf9412498d94692ab5956af3ad98ba3693a02d`.
The header also records `suite_topology_sha256`, the SHA-256 of the suite specification file.

## Convert a frozen model

`python -m isingmark_structured.convert` turns one research model (`.npz`) into a one-instance file.
The file has the same header as a generated file, with `generator` set to `frozen-model`, plus `problem_family`, `model_blake3` (the BLAKE3 digest of the `.npz` bytes), and `source_model`.
The converter maps the model to the fixture by qubit id, as the generator does.
It stops with an error on a value outside -1,000 to 1,000, on qubits that the suite specification lacks, and on a model with another number of spins.
The `reference` object of the instance holds the structure arrays of the model.
Node groups (`tetrahedra`, `clusters`, and `pairs` when every pair is a model coupler) appear as fixture node ids.
Every other array appears under `reference.unmapped`, unchanged, with `id_space` `suite`.

```bash
python -m isingmark_structured.convert path/to/model.npz --cell ice-glass-r8 \
  --problem-family ice-glass --source-model qpu-suite/work/cells/ice-glass-r8/models/<nonce>.npz
```

## Verify

The verify step needs the suite workspace, which exists only on the research host.

```bash
python -m isingmark_structured.verify --suite-dir /home/carback1/quip-data/regimes/qpu-suite
```

The command regenerates the 24 models of every family and compares `edges`, `j_milli` and `h_milli` with the saved suite models by exact array equality.
The comparison uses the suite numbering, before the fixture mapping.
The command also compares the extra arrays that both sides carry, like the tetrahedra.
It prints one row for each family and exits with status 1 when any model differs.

## Exactness

The port matches the suite generators bit for bit.
On 2026-10-15, `verify.py` matched 24 of 24 models for each of the 15 cells, and every shared extra array matched.
Exactness depends on the NumPy bit generator (PCG64 under `default_rng`) and on the SciPy `maximum_bipartite_matching` that the ice families use.
The verify run used NumPy 2.4.6 and SciPy 1.17.1.
Run the verify step again after you upgrade either package.

## Test and lint

```bash
pytest tests
ruff check .
pyright .
```
