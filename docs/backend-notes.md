# Backend notes

## Topology fixtures

`--topology-preset <name>` loads `fixtures/<name>.spec.json`.

| Preset | Nodes | Edges | Purpose |
|---|---|---|---|
| `smoke` | 8 | 8 | Fast checks |
| `cuopt-small` | 128 | 749 | Largest size `cuopt` passed every attempt |
| `common-small` | 548 | 4247 | Largest Advantage2 subgraph every backend accepts, including `beit` |
| `chimera-c8-16-4` | 1024 | 2976 | Chimera C(8,16). The only fixture BEIT's exact solver could accept |
| `chain-h0` | 4577 | 41515 | Chain h0 bucket. Pairs with the `chain-h0` corpus |
| `chain-ternary` | 4578 | 41531 | Chain ternary bucket. Pairs with the `chain-ternary` corpus |
| `advantage2-system1` | 4577 | 41515 | D-Wave Advantage2 (Zephyr), synthetic |
| `pegasus-p16` | 5640 | 40484 | Ideal Pegasus P16 |

`chain-h0` and `chain-ternary` are the two topologies the chain has actually
posed problems over. Each one is paired with a corpus of harvested instances
under `corpora/`. See `docs/datasets.md`.

`advantage2-system1` carries the `chain-h0` graph with the `chain-ternary`
allowed-h set, so it hashes to neither chain bucket and cannot replay either
corpus. It stays as a synthetic benchmark topology.

`common-small` and `cuopt-small` are connected induced subgraphs of
`advantage2-system1`, grown breadth-first from the highest-degree node. They
keep the original qubit identifiers, so both stay natively embeddable on
Advantage2 hardware.

`common-small` is sized to the `beit` limit. At 548 nodes and 4247 edges the
worst case is 4795 non-zero coefficients, which leaves 205 spare against the
5000 cap. The largest subgraph that fits is 579 nodes, which reaches exactly
5000 and leaves no margin.

`chimera-c8-16-4` is a complete Chimera C(8,16) graph: 128 unit cells of
K(4,4), so 1024 nodes, 2048 intra-cell edges and 928 inter-cell edges. Node
degree is 6 inside the grid and 5 at the edges. The worst case is 4000
non-zero coefficients, which leaves 1000 spare against the BEIT cap.

It is the one fixture shaped for BEIT's exact solver, which accepts Chimera
only and at most 8 by 16 unit cells. `quip-beit-sa` wraps the heuristic
`BEITUnconstrainedSampler` and not the exact `BEITSolver`, so nothing drives
that solver today. The fixture is what a comparison against it would need.

`pegasus-p16` comes from `dwave_networkx.pegasus_graph(16)` with node
identifiers remapped to `0..5639`. It is an ideal graph. A real
`Advantage_system1` has dead qubits, near 5436 working of 5640, so this
fixture is not natively embeddable on any specific machine. Replace it with
`DWaveSampler().to_networkx_graph()` output before running D-Wave.

`quip-cuda-sa` caps at 5000 nodes and cannot run `pegasus-p16`.

Measured behaviour of each registered backend. The registry in
`src/backend/registry.rs` lists what isingmark can drive. This file records
what each backend does in practice, including limits that the backend's own
`--capabilities` output does not report.

## Generated topologies

`--topology-generate complete:N` builds a complete graph with `N` nodes and
`N*(N-1)/2` edges. `--topology-generate circulant:N:D` builds a regular ring
with `N` nodes and `N*D/2` edges. Each node connects to `D/2` neighbors in
each direction. `N` must be positive. `D` must be even, nonnegative, and less
than `N`. Node ids start at zero. Generation needs no topology file.

Choose one of `--topology-generate`, `--topology-preset`, and `--topology-file`.
An explicit source flag overrides the topology source in a YAML config file.
The generator uses the same topology hash as a file containing the same model.
The hash includes the allowed coefficient sets.

`--couplings` applies only with `--topology-generate`:

| Value | Allowed J, milli | Allowed h, milli |
|---|---|---|
| `binary` (default) | `[-1000, 1000]` | `[-1000, 0, 1000]` |
| `int8` | Multiples of 16 from −992 to 992, excluding 0 | The same values, including 0 (125 levels) |
| `int32` | Every integer from −1000 to 1000, excluding 0 | Every integer from −1000 to 1000 |

All generated specs use spins `[-1000, 1000]`. Jobs carry exact milli values.
Float models cannot travel as milli jobs. Test float coefficients with the
miner's `bench run --model` interface.

## Multi-spin annealing

`cuda-msa` runs `quip-cuda-msa` with the sweeping interface.
Reads and sweeps control each job.
IsingMark passes `--max-nodes` from the topology size when it starts the miner.
The miner uses one SM per instance. Turing uses 64 reads. Devices with larger
shared-memory budgets use 128 reads.

The miner routes degree greater than 62 to its dense kernel. `complete:1410` uses
that path. The sparse path accepts `circulant:26260:60` and
`circulant:65797:20`. Limits depend on free device memory, coefficient
precision, and schedule length. Each model must pass the miner's memory check.
Shared memory controls how many spins stay on chip. The remaining spins use
device memory.

The miner session rejects topology messages larger than 4 MiB (4,194,304 bytes).
This transport limit caps generated topologies near these three sizes.
With `int8` coefficients, their messages use 3,797,256, 3,799,885, and
3,799,996 bytes, respectively. Binary messages use 1,452 fewer bytes.
These sizes leave at least 394,308 bytes below the session limit.
Measure larger models through `quip-cuda-msa bench`.

The throughput command exits nonzero if it completes zero jobs and rejects
at least one job. It still writes the records and summary for diagnosis.

## cuopt (quip-cuopt-milp)

`cuopt` is not competitive on this problem class, and it fails on some jobs
for reasons that are not size alone. The declared `max_nodes` of 20000 does
not reflect what the backend delivers.

### Measured results

Tested on induced subgraphs of `fixtures/advantage2-system1.spec.json`, with
`num_reads: 36` and cuOpt server 26.06 on an NVIDIA RTX A4000.

| Nodes | Edges | Jobs passed |
|---|---|---|
| 8 | 8 | 128 of 128 |
| 128 | 749 | 3 of 3 |
| 192 | 1270 | 2 of 2 |
| 256 | 1710 | 1 of 2 |
| 320 | 2298 | 2 of 2 |
| 384 | 2887 | 0 of 2 |
| 448 | 3345 | 1 of 2 |
| 548 | 4247 | 0 of 6 |

That table shows no size ceiling. 320 nodes passes cleanly while both 256 and
384 fail, so the failure is per job rather than per size. The rate
rises with node count, and 548 failed every attempt at time limits of 1, 10
and 60 seconds and at `CUOPT_GIGABYTES_PER_PROC` of both 1 and 10.

`fixtures/cuopt-small.spec.json` holds the 128-node case, the largest size
that passed every attempt.

### Quality and speed

At 128 nodes, `cuopt` takes 20.6 s per job. `cpu-sa` takes 55 ms per job on
the same problems and finds lower energy on all three. `cuopt` is 374 times
slower and gives worse results, on the size where it is most reliable.

cuOpt solves a mixed integer linear program. The miner converts the Ising
problem with a Fortet linearization, which gives a weak relaxation. On the
548-node problem the root relaxation objective is -4247 milli-units. The best
known energy from simulated annealing is -1665 milli-units. The bound starts
about 2.5 times away from the optimum, with 547 integer-infeasible variables.
The reported gap stays above 600 percent.

cuOpt has no native quadratic or QUBO mode, so the linearization is the only
available formulation. Mixed integer linear solvers are a poor match for dense
Ising problems. The problem class causes this result. Tuning does not change
it.

### Why jobs fail

The weak relaxation explains the poor energies. It does not explain why one
job at 320 nodes succeeds and another at 256 does not.

At 548 nodes the cuOpt solver process stops about 1.0 s into branch and bound
and then restarts, and the server answers the next poll with HTTP 500. The
miner maps that to `OVERLOADED`, which reads as load shedding rather than a
solver fault. An attempt to measure a pass rate over 10 jobs at 256 nodes hung
instead of finishing, which is the same HTTP 500 handling defect listed under
open defects below.

The evidence points at a reliability fault in the solver process or in the
miner's handling of a failed poll, not at a capacity limit. It is not
diagnosed.

### Time budget

`cuopt` is registered as `BackendKind::Annealing`, so isingmark sends the job
time limit in `anneal_time_us`. The default parameter curve interpolates
between 5 µs and 54 µs, which is a valid anneal time for a quantum annealer
and a meaningless time limit for a MILP solve.

Set `min_anneal_us` and `max_anneal_us` in a config file to give cuOpt a real
budget. Values near 1000000 give a one second limit per job. The miner's
`--time-limit-s` flag does not apply, because isingmark always sends a
non-zero `anneal_time_us`.

### Server setup

The cuOpt server needs a Python built against the system C library. A
Homebrew Python cannot load the system `libcuda.so.1`, and CUDA then reports
driver version 0. The failure appears as `cudaErrorInsufficientDriver`, which
names the wrong cause.

```bash
/usr/bin/python3 -m venv .cuopt
.cuopt/bin/pip install --extra-index-url https://pypi.nvidia.com cuopt-server-cu13
CUOPT_SERVER_PORT=5000 .cuopt/bin/python -m cuopt_server.cuopt_service
```

cuOpt is open source, so the wheels need no NGC account. The container image
at `nvcr.io` still needs one.

### Open defects

- `--check` passes while the solver is unusable. It probes `/cuopt/health`,
  which the API process answers without using the GPU.
- The miner maps every cuOpt error to `OVERLOADED`, which hides a solver
  failure behind a load-shedding code.
- One HTTP 500 response left the miner waiting with no work for 14 minutes
  instead of failing the job.

## beit (quip-beit-sa)

`--capabilities` reports `max_nodes: 5000` and `max_edges: 5000`. BEIT applies
one combined limit of 5000 non-zero coefficients, counting linear and
quadratic terms together. See `count_ising_nonzeros` in the miner source. A
problem at both advertised caps has 10000 non-zero coefficients and is
rejected.

For a topology with `n` nodes and `e` edges, the usable condition is
`n + e <= 5000`. A random 3-regular graph reaches 2000 nodes. An induced
subgraph of Advantage2 reaches 579 nodes.
