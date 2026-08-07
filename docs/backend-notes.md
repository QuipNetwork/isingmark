# Backend notes

## Topology fixtures

`--topology-preset <name>` loads `fixtures/<name>.spec.json`.

| Preset | Nodes | Edges | `allowed_h_milli` | Purpose |
|---|---|---|---|---|
| `smoke` | 8 | 8 | ternary | Fast checks |
| `cuopt-small` | 128 | 749 | ternary | Largest size `cuopt` passed every try |
| `common-small` | 548 | 4247 | ternary | Largest Advantage2 subgraph every backend accepts, including `beit` |
| `chimera-c8-16-4` | 1024 | 2976 | ternary | Chimera C(8,16). The only fixture BEIT's exact solver could accept |
| `chain-h0` | 4577 | 41515 | `[0]` | Advantage2_system1 working graph. Pairs with the `chain-h0` corpus |
| `advantage2-system1` | 4577 | 41515 | ternary | The same working graph with fields. `extends: chain-h0` |
| `chain-ternary` | 4578 | 41531 | ternary | Older Advantage2 graph. Pairs with the `chain-ternary` corpus |
| `advantage-system6` | 5612 | 40088 | ternary | Advantage_system6 working graph (Pegasus) |
| `pegasus-p16` | 5640 | 40484 | ternary | Ideal Pegasus P16 |

Every one of those graphs also ships an `-h0` variant with
`allowed_h_milli: [0]`. See "The h0 regime" below.

`chain-h0` and `chain-ternary` are the two topologies the chain has actually
posed problems over. Each one is paired with a corpus of harvested instances
under `corpora/`. See `docs/datasets.md`.

### Which fixtures are real machines

`chain-h0` and `advantage-system6` were read from the live QPUs on 2026-08-07
through `dwave.cloud`. Each one is an exact working graph, qubit for qubit and
coupler for coupler:

| Fixture | Solver | Topology | Nominal qubits | Working |
|---|---|---|---|---|
| `chain-h0` | `Advantage2_system1` | Zephyr [12, 4] | 4800 | 4577 |
| `advantage-system6` | `Advantage_system6` | Pegasus [16] | 5760 | 5612 |

`advantage2-system1` is the same `Advantage2_system1` working graph as
`chain-h0`, carrying ternary fields instead of `[0]`. The two are the h0 and
non-h0 pair for that machine. It declares `extends: chain-h0` so one graph
cannot drift away from the other. Its `topology_hash` matches neither chain
bucket, because the chain never posed ternary-field problems over this graph,
so it cannot replay either corpus.

`pegasus-p16` is an ideal `dwave_networkx.pegasus_graph(16)` with identifiers
remapped to `0..5639`. It matches no real machine. Prefer
`advantage-system6` for anything meant to run on Pegasus hardware.

`common-small` and `cuopt-small` are connected induced subgraphs of the
Advantage2 working graph, grown breadth-first from the highest-degree node.
They keep the original qubit identifiers, so both stay natively embeddable on
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
identifiers remapped to `0..5639`. It is an ideal graph, so it is not natively
embeddable on any specific machine. `advantage-system6` is the real
Pegasus graph and should be preferred for hardware runs.

### The h0 regime

Every graph ships an `-h0` variant that sets `allowed_h_milli: [0]` and
changes nothing else.

`h = 0` is the regime worth measuring. With no local fields the problem is a
pure spin glass: the ground state is set entirely by frustration among the
couplings, there is no field to bias a spin toward an easy answer, and the
`+s` and `-s` states are degenerate. Adding fields gives a solver terms it can
satisfy greedily, which makes an instance easier and makes engines look better
than they are. The chain's own hardest bucket is the h0 one, so h0 is also the
regime that matches production mining.

| Base | h0 variant |
|---|---|
| `smoke` | `smoke-h0` |
| `cuopt-small` | `cuopt-small-h0` |
| `common-small` | `common-small-h0` |
| `chimera-c8-16-4` | `chimera-c8-16-4-h0` |
| `pegasus-p16` | `pegasus-p16-h0` |
| `advantage-system6` | `advantage-system6-h0` |
| `chain-ternary` | `chain-ternary-h0` |
| `advantage2-system1` | `chain-h0` |

`advantage2-system1` is the one case with no `-h0` suffix, because its h0 form
already exists under the name `chain-h0`, which is also the corpus pairing
name. Adding a second file for the same topology would give one graph two
names and two chances to drift.

An h0 variant is a two-line file:

```json
{"extends": "chain-ternary", "allowed_h_milli": [0]}
```

`extends` names a bundled fixture and inherits its `nodes` and `edges`. Any
allowed-value set the variant omits is inherited too. Inheritance is one level
deep, and a spec that sets `extends` must not also carry its own `nodes` or
`edges`. This keeps a graph described exactly once. Duplicating it is how
`advantage2-system1` came to hold one bucket's graph with another bucket's
fields.

## D-Wave hardware ranges

Read from the live solvers on 2026-08-07. Reading solver properties costs no
QPU time.

| Property | `Advantage2_system1` | `Advantage_system6` |
|---|---|---|
| `h_range` | `[-6.0, 6.0]` | `[-4.0, 4.0]` |
| `j_range` | `[-1.0, 1.0]` | `[-1.0, 1.0]` |
| `extended_j_range` | `[-2.0, 1.0]` | `[-2.0, 1.0]` |
| `per_group_coupling_range` | `[-13.0, 10.0]` | not applicable |
| `per_qubit_coupling_range` | not applicable | `[-18.0, 15.0]` |
| `annealing_time_range` | `[0.5, 2000.0]` µs | `[0.5, 2000.0]` µs |
| `num_reads_range` | `[1, 10000]` | `[1, 10000]` |

`allowed_h_milli` and `allowed_j_milli` in a topology spec are the chain
protocol's discrete draw sets, not hardware limits. They feed the
`topology_hash`, so widening them changes problem identity. The comparison
below is about headroom, not about a defect.

- **Fields use one sixth of the range.** The chain draws `h` from
  `{-1, 0, +1}` against a `[-6, 6]` bias range. The h0 bucket uses none of it.
- **Couplings are saturated.** The chain draws `J` from `{-1, +1}`, which is
  exactly `j_range`. No headroom is left without `extended_j_range`.
- **Group coupling sums reach the limit.** Zephyr splits each qubit's couplers
  into two groups and bounds the signed sum of `J` in each. The largest group
  on the Advantage2 working graph holds 10 couplers, and 4829 of 9154 groups
  are that size. At `|J| = 1` an all-positive group sums to exactly `+10`,
  which is the positive limit. That happens with probability 2⁻¹⁰ per full
  group, so roughly 5 groups per instance sit precisely on the boundary. The
  limit is never exceeded, but the positive side has no margin.

`quip-miner-dwave` decodes milli straight to floats and passes `h` and `J`
through unscaled. It sets only `num_reads` and optionally `annealing_time`,
and never sets `auto_scale`, so D-Wave's default applies and that default is
on. Every submission is rescaled to fill `h_range` and
`extended_j_range`. Rescaling shrinks the problem's energy scale against a
fixed hardware noise floor. Nobody has measured what that costs in solution
quality. Reproduce the properties above with:

```python
from dwave.cloud import Client
with Client.from_config(token=...) as c:
    s = [x for x in c.get_solvers() if x.id.startswith("Advantage2_system1")][0]
    print(s.properties["h_range"], s.properties["per_group_coupling_range"])
```

`quip-cuda-sa` caps at 5000 nodes and cannot run `pegasus-p16`.

Measured behaviour of each registered backend. The registry in
`src/backend/registry.rs` lists what isingmark can drive. This file records
what each backend does in practice, including limits that the backend's own
`--capabilities` output does not report.

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
rises with node count, and 548 failed every try at time limits of 1, 10
and 60 seconds and at `CUOPT_GIGABYTES_PER_PROC` of both 1 and 10.

`fixtures/cuopt-small.spec.json` holds the 128-node case, the largest size
that passed every try.

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
solver fault. A try at measuring a pass rate over 10 jobs at 256 nodes hung
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
