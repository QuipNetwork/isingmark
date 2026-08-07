<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Chain datasets

isingmark can replay problems the chain actually posed, rather than deriving
fresh ones from a seed. Two corpora ship with the crate. Each one holds
instances harvested from chain. Each one is paired with the topology those
instances were drawn against.

```bash
isingmark throughput --backend cpu-sa \
  --topology-preset chain-h0 --corpus chain-h0 --duration 10m
```

## What ships

| Corpus | Instances | Paired topology | Nodes | Edges | `allowed_h_milli` |
| --- | --- | --- | --- | --- | --- |
| `chain-h0` | 4658 | `chain-h0` | 4577 | 41515 | `[0]` |
| `chain-ternary` | 3148 | `chain-ternary` | 4578 | 41531 | `[-1000, 0, 1000]` |

The corpora are in `corpora/`, the topologies in `fixtures/`. Both came from
`quip-coordinator download`, which scanned 7806 chain solutions and bucketed
them by `topology_hash`. Every instance verified against its block.

The two buckets are different graphs, not one graph under two field settings.
`chain-ternary` is exactly `chain-h0` plus qubit 4189 and the 16 couplers that
touch it. The h0 bucket carries the alias `chain-default` upstream.

## `chain-h0` is the machine

`chain-h0` matches the `Advantage2_system1` working graph exactly, as read
from the live solver on 2026-08-07: 4577 qubits and 41515 couplers, node for
node and coupler for coupler. Nothing needs embedding to run it.

`advantage2-system1` is the same working graph with ternary fields instead of
`[0]`. The two are the h0 and non-h0 pair for that machine.

## The ternary corpus has a retired qubit

**Qubit 4189 is no longer in the Advantage2_system1 working set.** It was
alive when the ternary bucket was harvested and calibration has since retired
it. `chain-ternary` is not natively embeddable on the machine today.

This blocks a QPU run against the ternary corpus. Both ways out change the
problem:

- Drop 4189 and its 16 couplers. That is a different graph, so a different
  `topology_hash`, so the drawn problems no longer match the corpus.
- Minor-embed 4189 onto a chain of physical qubits. That changes the energy
  scale and adds chain-break post-processing.

The `chain-h0` corpus is unaffected. Prefer it for any comparison that has to
run on hardware.

Re-check before a QPU run, because the working set moves with calibration:

```python
from dwave.cloud import Client
with Client.from_config(token=...) as c:
    s = [x for x in c.get_solvers() if x.id.startswith("Advantage2_system1")][0]
    print(sorted(set(json.load(open("fixtures/chain-ternary.spec.json"))["nodes"]) - set(s.nodes)))
```

Each corpus is ranked hardest first, by the lowest energy any miner achieved:

| Corpus | Best energy (milli) | Worst energy (milli) |
| --- | --- | --- |
| `chain-h0` | -14649000 | -13953000 |
| `chain-ternary` | -15060000 | -14271000 |

## Corpus format

One JSON object per line.

```json
{"nonce":"982178...56e0","energy_milli":-14649000,"max_energy_milli":-14534320,"qblock_id":4058}
```

| Field | Meaning |
| --- | --- |
| `nonce` | 64 hex characters. The only entropy for the drawn problem. Required. |
| `energy_milli` | Best energy any miner reached on chain. The bar to beat. |
| `max_energy_milli` | The difficulty gate. A solution at or below this passed. |
| `qblock_id` | Block that recorded the winning solution. Provenance only. |

Only `nonce` is required. Pass a path instead of a name to run your own file.

The format matches the nonce-ref half of `quip-coordinator drive --source
list`, so a corpus file works with either tool.

## How replay works

A nonce plus a topology gives back one problem. `draw_ising_milli` takes the
nonce and the topology's node count, edge count, and allowed-value sets, then
derives every `h` and `J` value. Same nonce plus same topology always gives
the same problem, which is what makes a chain instance replayable at all.

The job id is the nonce, so records from a corpus run join directly against
the corpus file on `job_id`. That is how you compare an engine's energy to
`energy_milli` per instance.

## Pairing is enforced

A corpus does not carry its topology. Replaying `chain-h0` against the
`chain-ternary` graph draws valid-looking problems that are not the ones the
chain recorded, and every energy you measure would be meaningless.

isingmark refuses that pair. A bundled corpus is checked against the
`topology_hash` of the fixture it belongs to, and a mismatch fails before any
job runs:

```
failed to load corpus: corpus "chain-h0" was drawn against topology e66d3dfa...,
but the run supplies 6eae38a9.... Pair it with the matching --topology-preset,
or the replayed problems are not the ones the chain recorded
```

A corpus loaded from a path declares no pairing, so that check does not apply.
Pairing it correctly is the caller's job.

`advantage2-system1` is not a chain bucket. It is the real Advantage2 working
graph, the same one `chain-h0` carries, but with ternary fields. The chain
never posed ternary-field problems over this graph, so it hashes to neither
bucket and cannot replay either corpus. `tests/topology.rs` pins all three
hashes.

## A corpus run ends when the corpus does

A corpus is finite and yields each instance once. `--duration` becomes an
upper bound: the run stops at the duration or at the last instance, whichever
comes first. Read `measured_wall_ms` in the summary to tell which happened.

The summary records the source of every run:

| Field | Seeded run | Corpus run |
| --- | --- | --- |
| `corpus` | `null` | the name or path |
| `job_source` | `seed:42` | the name or path |
| `corpus_instances` | `null` | corpus size |

`seed` is still reported on a corpus run, but it did not apply.

## Regenerating

The corpora are trimmed copies of a `quip-coordinator download` run. To
refresh them, download again and keep the four fields above:

```bash
quip-coordinator download --out hardest_models
# one directory per topology_hash, each with instances.jsonl and topology.spec.json
```

Copy each bucket's `topology.spec.json` to `fixtures/<name>.spec.json` and add
the hash to `chain_fixtures_hash_to_their_recorded_bucket` in
`tests/topology.rs`. That test is what catches a bucket copied to the wrong
name.

## The h0 regime

`h = 0` is the regime worth measuring, and it is the one the chain's hardest
bucket uses. With no local fields the problem is a pure spin glass. The ground
state comes entirely from frustration among the couplings, and no field biases
a spin toward an easy answer. Adding fields hands a solver terms it can satisfy
greedily, which makes an instance easier.

Every bundled graph ships an `-h0` variant, so the h0 regime is available at
every scale rather than only at chain scale. See "The h0 regime" in
`docs/backend-notes.md` for the full list and for how `extends` works.

## Limits

- Only `throughput` mode reads `--corpus` today. The other modes derive from a
  seed.
- `chain-ternary` cannot run natively on `Advantage2_system1` anymore. See the
  retired-qubit section above.
- `quip-cuda-sa` and `quip-cuda-gibbs` cap below 4577 nodes by default. Raise
  the cap with `--max-nodes`, up to what the card's memory allows. See
  `docs/backend-notes.md`.
- `beit` cannot accept either topology. Its limit is 5000 combined non-zero
  coefficients, and these problems carry more than 46000.
