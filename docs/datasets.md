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

`sweep` accepts the same flag. Every cell reloads the corpus from the start,
so all cells measure the identical instances and a corpus caps `--num-jobs`.

Explicit-coefficient instances, such as the structured families of the QPU
suite, replay through `--instances` instead. See `docs/structured-datasets.md`.

## What ships

`corpora/` holds two kinds of file. The two chain corpora below are bundled and
replay through `--corpus`. The other files are frozen-model examples, one per
problem cell, and replay through `--instances`. They are not bundled. The
cells are listed in `problems/README.md`.

| Corpus | Instances | Paired topology | Nodes | Edges | `allowed_h_milli` |
| --- | --- | --- | --- | --- | --- |
| `chain-h0` | 4658 | `chain-h0` | 4577 | 41515 | `[0]` |
| `chain-ternary` | 3148 | `chain-ternary` | 4578 | 41531 | `[-1000, 0, 1000]` |

The corpora are in `corpora/`, the topologies in `fixtures/`. Both came from
`quip-coordinator download`, which scanned 7806 chain solutions and bucketed
them by `topology_hash`. Every instance verified against its block.

The two buckets are different graphs, not one graph under two field settings.
`chain-ternary` has one more node (id 4189) and 16 more edges. The h0 bucket
carries the alias `chain-default` upstream.

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

`advantage2-system1` is not a chain bucket. It holds the `chain-h0` graph with
the `chain-ternary` allowed-h set, so it hashes to neither, and it cannot
replay either corpus. It stays as a synthetic benchmark topology.
`tests/topology.rs` pins all three hashes.

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

## Limits

- Only `throughput` mode reads `--corpus` today. The other modes derive from a
  seed.
- `quip-cuda-sa` and `quip-cuda-gibbs` cap below 4577 nodes by default. Raise
  the cap with `--max-nodes`, up to what the card's memory allows. See
  `docs/backend-notes.md`.
- `beit` cannot accept either topology. Its limit is 5000 combined non-zero
  coefficients, and these problems carry more than 46000.
