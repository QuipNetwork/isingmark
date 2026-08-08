# Backend notes

What each registered backend does in practice, including limits its own
`--capabilities` output does not report. The registry in
`src/backend/registry.rs` lists what isingmark can drive.

Every measured number here was re-taken on 2026-08-07 unless the text says
otherwise. `beit` is the exception: it has never run, because it needs a paid
key and cannot accept a chain-scale problem anyway.

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

`quip-cuda-sa` defaults to 5000 nodes, but isingmark passes the topology's own
node count through `--max-nodes`, so the default is not a ceiling. Both
`pegasus-p16` (5640 nodes) and `advantage-system6` (5612) ran on `cuda` in the
2026-08-07 round. The real limit is the card's memory, and the miner reports
an error rather than failing quietly when a problem does not fit.

## Measured throughput

Measured 2026-08-07 with one miner session per run, so subprocess spawn,
device open and kernel load are paid once instead of once per batch. Read
"Removing startup cost" and "Measurement conditions" below before comparing
these numbers to anything measured elsewhere.

Throughput mode runs repeated sessions until the duration elapses, and each
session pays that fixed cost. `--batch-size` controls how many jobs share a
session. The default of 8 measures throughput as a coordinator that restarts
miners would see it. A large batch measures the backend.

### Removing startup cost

Same runs, same host, batch 8 against batch 512:

| Topology | Backend | batch 8 | batch 512 | Ratio |
|---|---|---|---|---|
| `smoke-h0` (8) | `cpu-sa` | 2890 | 267842 | 93x |
| `smoke-h0` (8) | `cuda` | 182 | 12883 | 71x |
| `chain-h0` (4577) | `cpu-sa` | 91.6 | 267.3 | 2.9x |
| `chain-h0` (4577) | `cpu-gibbs` | 44.3 | 150.0 | 3.4x |
| `chain-h0` (4577) | `cuda` | 72.7 | 86.7 | 1.2x |

Startup dominated every small-topology figure and still cost a factor of
three at chain scale for the CPU backends. Any earlier throughput number
taken at the default batch understates the backend, most severely for GPU
backends and small problems.

`median_device_ms` in a run summary is the per-job device time. It excludes
session setup for every backend, including ones that cannot take a large
batch, so it is the metric to use when batch size cannot be raised.

### Size ladder, h0 regime

30 s per run, hardness 0.5, batch 512.

| Topology | Nodes | `cpu-sa` | `cpu-gibbs` | `cuda` |
|---|---|---|---|---|
| `smoke-h0` | 8 | 267842 | 99651 | 12883 |
| `cuopt-small-h0` | 128 | 11343 | 5610 | 4832 |
| `common-small-h0` | 548 | 2473 | 1321 | 1671 |
| `chimera-c8-16-4-h0` | 1024 | 1829 | 872 | 1163 |
| `advantage-system6-h0` | 5612 | 254 | 136 | 85 |
| `pegasus-p16-h0` | 5640 | 254 | 135 | 86 |

Jobs per minute. `cpu-sa` leads at every size on this host.

Median device time per job at 4577 nodes: `cpu-sa` 3334 ms, `cuda` 5617 ms,
`cpu-gibbs` 5937 ms. `cuda` also shows 11105 ms of median wall against that
5617 ms of device time, so about half its per-job cost sits outside the
kernel even in a single session.

### h0 against fields, on one graph

`chain-h0` and `advantage2-system1` are the same 4577-node Advantage2 working
graph. One draws `h` from `{0}`, the other from `{-1, 0, +1}`. 60 s, hardness
0.5, batch 512.

| Backend | h0 jobs/min | fields jobs/min | h0 median energy | fields median energy |
|---|---|---|---|---|
| `cpu-sa` | 267.3 | 271.0 | -14268000 | -14728000 |
| `cpu-gibbs` | 150.0 | 152.3 | -14272000 | -14734500 |
| `cuda` | 86.7 | 86.7 | -14267000 | -14730000 |

**Throughput is the same in both regimes.** An earlier round at batch 8
reported h0 as the faster one, by 91.6 against 75.8 for `cpu-sa`. That gap was
session-startup noise and it disappears once the startup is paid once.

The energy difference is real and holds. Fields reach about 460000 milli
lower on every backend, near 3.2 percent. The field regime is easier, not
faster. A solver gains 2998 field terms it can satisfy greedily and reaches
deeper energy on a problem that costs the same to compute.

That is the reason to benchmark at h0, and it sizes the hazard of replaying an
h0 corpus against a fields topology: the run would report energies about 3
percent better than the real problem allows.

### cuda-gibbs is missing from these tables

`quip-cuda-gibbs` hangs, and the hang is what raising the batch size exposed.
See "cuda-gibbs session hang" below. Its 2026-08-07 batch-8 figure of 184
jobs per minute at 4577 nodes could not be reproduced later the same day, so
it is not reported here.

### Measurement conditions

Host `finney`: 32 cores, 2 NVIDIA RTX A4000, 125.7 GiB RAM.

Load average ran between 14 and 30 during this round. An earlier round the
same day ran at 44 to 92, because another user's job held roughly 14 of the
32 cores. Absolute CPU numbers are not comparable between the two rounds for
that reason as well as the batching change. Comparisons within a round hold,
because every backend saw the same conditions.

GPU numbers are cleaner. Both cards were idle apart from these runs.

Scripts and raw output are under `out/rerun3-2026-08-07/`.

## cuda-gibbs session hang

**Root caused and fixed.** Two separate defects in `kernels/gibbs.cu`, both of
the same shape: the kernel retired a nonce rather than waiting for the host to
finish uploading to it. Fixes are on `fix/gibbs-cold-start-claim` in
`quip-miner-cuda`, not yet released.

### The cold-start race

The host enqueues each cold-start slot-0 upload on the transfer stream, data
first and the `SLOT_READY` ctrl flag last, then launches the persistent kernel
on the compute stream. The two streams are deliberately unordered. At large
`N` the uploads are still in flight when the kernel starts, so the lead
block's single-pass scan saw EMPTY, wrote `CTRL_GENERATION = -1`, and every
block of that nonce returned. The READY flag landed microseconds later with no
block left to claim it, and `pump_session` polled for a `SLOT_COMPLETE` that
could never arrive.

Confirmed by direct observation rather than inference. Host-side dumps of the
ctrl mailbox on stall showed the same signature in all 12 hangs examined:

```
nonce=7 slots=[1,0,0] active_slot=0 blocks_done=0 work_queue=0
        exit_now=0 generation=-1 host_active=Some(0)
```

`generation=-1` is the nonce retired at birth, `slots=[1,0,0]` is slot 0 READY
and unclaimed, `work_queue=0` means it never processed anything. Healthy
nonces in the same dump read `generation=1`, `slots=[3,0,0]`, `work_queue=8`.
The dead nonce was always a high-numbered one, which is what upload order
predicts.

This explains every earlier observation. The GPU is idle because the nonce
never runs, rather than spinning as a lost barrier would. The window grows
with `N` because the upload is larger, which is the size dependence.
`compute-sanitizer` hid the bug because it serializes copies and launches,
closing the window.

### The feeder-starvation bug

Separately, the next-slot scan had the same single-pass flaw, so a nonce could
retire mid-session whenever the feeder was briefly empty. `kernels/sa.cu`
already waits through this and documents why (QUI-828); both kernels came from
the same verbatim v0.2 copy and only SA carried the fix. This is what made
`--batch-size` above about 18 hang at any size.

### Verification

Both arms interleaved in one session, 4096 nodes, batch 8:

| Build | Hangs | Sequence |
|---|---|---|
| before | 8 of 20 | `.HHH..H.H..H.HH.....` |
| after | 0 of 20 | `....................` |

Reproduced here independently, building both arms from source and
interleaving them. The fixed build took 0 hangs of 14 and the unfixed 1 of
14, which puts the fixed build at 0 hangs in 34 attempts across both runs.

Baseline rates are unstable between sessions, at 6 of 12, 8 of 20 and 1 of 14
on different days. The mailbox evidence rather than those rates is what
establishes the mechanism.

Also verified after the fix: 8 of 8 at 4577 nodes and 4 of 4 at 5612. The
8-node batch-64 case that previously hung 11 times in 12 passed 8 of 8.

### The host no longer waits forever

`pump_session` now carries a 30 s no-completion deadline
(`QUIP_CUDA_PUMP_DEADLINE_MS`, 0 disables). On expiry it dumps the ctrl
mailbox, rejects held jobs as `Overloaded`, which is retryable, and tears the
session down. Verified against the unfixed kernel, where 10 of 10 runs exited cleanly and 6
of those recovered from a wedge that previously hung forever.

This is worth having whatever else changes. A rejected job is visible in the
records, unlike a hang.

### Still open

`kernels/sa.cu` has the identical single-scan initial claim and is latently
exposed to the same cold-start race. It was left untouched and SA hang rates
were not measured, so "SA never hangs" is an observation rather than a
guarantee.

## cuopt (quip-cuopt-milp)

`cuopt` is not competitive on this problem class, and it fails on some jobs
for reasons that are not size alone. The declared `max_nodes` of 20000 does
not reflect what the backend delivers.

### Measured results

Two measurement rounds, both with `num_reads: 36` and cuOpt server 26.06 on an
NVIDIA RTX A4000. The 2026-08-06 round used ad-hoc induced subgraphs of the
Advantage2 graph. The 2026-08-07 round used the committed fixtures and a 10
second time limit.

| Nodes | Edges | 2026-08-06 | 2026-08-07 |
|---|---|---|---|
| 8 | 8 | 128 of 128 | 32 of 32 |
| 128 | 749 | 3 of 3 | 3 of 3 |
| 192 | 1270 | 2 of 2 | not retested |
| 256 | 1710 | 1 of 2 | not retested |
| 320 | 2298 | 2 of 2 | not retested |
| 384 | 2887 | 0 of 2 | not retested |
| 448 | 3345 | 1 of 2 | not retested |
| 548 | 4247 | **0 of 6** | **5 of 6** |

**The 548-node result reversed.** Same size, same parameters, same server
version, a restarted server process: 0 of 6 became 5 of 6. The one failure was
`OVERLOADED`, the same code the earlier round saw on every job.

That settles the reading of the first table. No size ceiling exists, and no
size makes the backend reliably fail. Failures are per job,
they come and go across server restarts, and a small sample at one size says
almost nothing. The intermediate sizes were ad-hoc subgraphs with no committed
fixture, so they were not re-measured.

Six jobs is a small sample. Treat 5 of 6 as "sometimes works at 548", not as a
rate.

`fixtures/cuopt-small.spec.json` holds the 128-node case, which has passed
every try in both rounds.

### Quality and speed

Measured 2026-08-07 with `num_reads: 36`, a 10 second cuOpt time limit, and
`num_sweeps: 550` for `cpu-sa`. Both backends saw the same fixtures.

| Nodes | Backend | Wall per job | Passed | Median energy (milli) |
|---|---|---|---|---|
| 128 | `cuopt` | 22.99 s | 3 of 3 | -313 |
| 128 | `cpu-sa` | 0.254 s | 200 of 200 | -323 |
| 548 | `cuopt` | 27.53 s | 5 of 6 | -803 |
| 548 | `cpu-sa` | 1.019 s | 200 of 200 | -1618 |

`cuopt` is 90 times slower at 128 nodes and 27 times slower at 548. The
earlier round reported 374 times at 128 nodes. `cuopt` did not get faster
between the rounds (20.6 s then, 22.99 s now). `cpu-sa` got slower, from
55 ms to 254 ms, because the 2026-08-07 round ran on a heavily loaded host.
See "Measurement conditions" below. Use the ratio as an order of magnitude,
not a figure.

Quality separates the two sizes. At 128 nodes `cuopt` is close, about 3
percent above `cpu-sa`'s median. At 548 nodes it reaches -803 against -1618,
roughly half the depth. The gap widens with problem size, which is what the
relaxation below predicts.

cuOpt solves a mixed integer linear program. The miner converts the Ising
problem with a Fortet linearization, which gives a weak relaxation. Measured
`mip_gap` and bound:

| Nodes | `mip_gap` | Solution bound | Best known from SA | Bound distance |
|---|---|---|---|---|
| 8 | 0.0 | -11 | -13 | solved to optimality |
| 128 | 1.13 | -618 | -345 | 1.8 times |
| 548 | 6.09 | -4089 | -1663 | 2.46 times |

At 548 nodes the bound starts about 2.5 times away from the best known energy
and the gap stays above 600 percent, which both rounds agree on. At 8 nodes
cuOpt closes the gap completely, so the formulation is correct. It does not
scale.

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

The 2026-08-07 round strengthens that reading. A restarted server passed 5 of
6 jobs at the same 548 nodes that had failed 6 of 6, and the single failure
still came back as `OVERLOADED`. Whatever the fault is, it survives across
sizes and clears across server restarts. Nothing about the problem size
predicts it.

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
