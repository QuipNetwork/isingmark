# Backend notes

## Topology fixtures

`--topology-preset <name>` loads `fixtures/<name>.spec.json`.

| Preset | Nodes | Edges | Purpose |
|---|---|---|---|
| `smoke` | 8 | 8 | Fast checks |
| `advantage2-system1` | 4577 | 41515 | D-Wave Advantage2 (Zephyr) |
| `advantage2-sub` | 548 | 4247 | Largest Advantage2 subgraph every backend accepts, including `beit` |
| `advantage2-sub128` | 128 | 749 | Reproduces the `cuopt` size ceiling below |
| `chimera-c8-16-4` | 1024 | 2976 | Chimera C(8,16). The only fixture BEIT's exact solver could accept |
| `pegasus-p16` | 5640 | 40484 | Ideal Pegasus P16 |

`advantage2-sub` and `advantage2-sub128` are connected induced subgraphs of
`advantage2-system1`, grown breadth-first from the highest-degree node. They
keep the original qubit identifiers, so both stay natively embeddable on
Advantage2 hardware.

`advantage2-sub` is sized to the `beit` limit. At 548 nodes and 4247 edges the
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

## cuopt (quip-cuopt-milp)

Do not use `cuopt` for problems above about 128 nodes. The declared
`max_nodes` of 20000 does not reflect what the backend can solve.

### Measured results

Tested on induced subgraphs of `fixtures/advantage2-system1.spec.json`, with
`num_reads: 36` and cuOpt server 26.06 on an NVIDIA RTX A4000.

| Nodes | Edges | Time limit | Result |
|---|---|---|---|
| 8 | 8 | 103 µs | 128 of 128 jobs passed |
| 128 | 749 | 10 s | 3 of 3 jobs passed, energy worse than `cpu-sa` on all 3 |
| 548 | 4247 | 1 s, 10 s, 60 s | 0 of 6 jobs passed, solver process stops |

At 128 nodes, `cuopt` takes 20.6 s per job. `cpu-sa` takes 55 ms per job on
the same problems and finds lower energy on all three. `cuopt` is 374 times
slower and gives worse results.

At 548 nodes the cuOpt solver process stops about 1.0 s into branch and
bound, then restarts. The failure does not depend on the time limit or the
memory pool size. Tests at `CUOPT_GIGABYTES_PER_PROC` of 1 and 10 both fail.
The miner reports the failure as `OVERLOADED`.

### Cause

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
