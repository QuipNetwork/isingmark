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

`corpora/` holds three kinds of file. The two chain corpora below are bundled and
replay through `--corpus`. `smoke-ice-glass.jsonl` is a two-instance smoke file on the
`smoke` fixture. The other files are frozen-model examples, one per
problem cell, and replay through `--instances`. Three cells have no example.
`cbfm-p-f17` has no frozen model, and `sports-break` and `paint-shop` use their
own graphs. The examples are not bundled. The cells are listed in
`problems/README.md`.

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

## Archived research fixtures

An archived fixture records instances that came from research runs, with the
witnesses that the research kept. One JSON file holds one cell, at
`problems/<family>/fixtures/<cell>.json`. Numbers follow the SC-2 encoding:
hex is lowercase without `0x`, every 64-bit value is a decimal string, and
small counts are JSON integers.

```json
{
  "schema_revision": 1,
  "provenance": "archived-research",
  "about": ["at least one string"],
  "sources": [{ "path": "<repo-relative file>", "blake3": "<64 hex>" }],
  "archived_instances": [
    {
      "name": "<unique within the file>",
      "cell": "<cell id>",
      "nonce": "<64 hex>",
      "archive_origin": "<research-root-relative path>",
      "archive_blake3": "<64 hex>",
      "source_recipe": { "source": "<generator id in problems/sources.json>", "name": "<recipe>" },
      "node_order": "physical or ascending",
      "node_ids": [907, 41, 12],
      "edges": [[0, 1], [0, 2]],
      "h": [0, -500, 31],
      "j": [-1000, 62],
      "scale": 1000,
      "offset_milli": "0",
      "qubo_offset_milli": null,
      "family_id": null,
      "descriptor_digest": null,
      "params_hex": null,
      "postprocess_policy": { "family": "ice-tiles", "method": "anneal:b0=2", "budget_s": 0.25 },
      "witnesses": [
        { "name": "qpu-best", "spins": [1, -1, 1], "claimed_energy_milli": "1593" },
        { "name": "pp-state", "missing": "state not stored in the archive" }
      ]
    }
  ]
}
```

The validator reports every violation, not only the first. Each message names
the file and the instance. The rules are:

- `schema_revision` is `1`, `provenance` is `archived-research`, and `about`
  is not empty. The validator rejects unknown keys at every level.
- Each `sources[].path` is a safe relative path with `/` separators and no
  empty, `.`, or `..` part. The file exists in the repository, and its BLAKE3
  hash equals `blake3`.
- Instance names are unique within the file.
- `nonce` and `archive_blake3` are 64 lowercase hex characters.
  `archive_origin` is a safe relative path. The validator does not check that
  it exists, because the research archive is not in the repository. It checks
  the form of `archive_blake3` only.
- `source_recipe.source` names a source of kind `generator` in
  `problems/sources.json`, and `source_recipe.name` is not empty.
- `node_ids` has no duplicates. With `node_order` `ascending`, the ids increase
  at every step. No other `node_order` value is valid.
- Each edge `[a, b]` has `a < b < len(node_ids)`, so no edge is a self-loop.
  The edge list increases in lexicographic order, so it is sorted and has no
  duplicates.
- `h` has one entry per node, `j` has one entry per edge, and `scale` is
  `1000`.
- `offset_milli`, `qubo_offset_milli` when it is not null, and
  `claimed_energy_milli` are decimal strings that parse as 64-bit integers. A
  JSON number in any of these fields fails validation.
- `family_id`, `descriptor_digest`, and `params_hex` are each null or valid:
  a non-empty string, 64 lowercase hex characters, and even-length lowercase
  hex.
- `postprocess_policy` has a non-empty `family` and `method`, and `budget_s`
  is zero or more.
- A witness has either `spins` with `claimed_energy_milli`, or a `missing`
  reason. It never has both and never has neither. Witness names are unique
  within the instance.
- Every spin is `1` or `-1`, and `spins` has one entry per node. The energy
  `offset_milli + sum(h[i] * s[i]) + sum(j[e] * s[a] * s[b])`, computed in
  128-bit integers, equals `claimed_energy_milli`. `qubo_offset_milli` is
  metadata and does not enter the energy.

The validator lives in `tests/campaign_import_fixtures.rs`. Run it with:

```bash
cargo test --test campaign_import_fixtures
```

The test `recorded_fixtures_are_valid` checks every file that matches
`problems/*/fixtures/*.json` against the real `problems/sources.json`.

## Limits

- Only `throughput` mode reads `--corpus` today. The other modes derive from a
  seed.
- `quip-cuda-sa` and `quip-cuda-gibbs` cap below 4577 nodes by default. Raise
  the cap with `--max-nodes`, up to what the card's memory allows. See
  `docs/backend-notes.md`.
- `beit` cannot accept either topology. Its limit is 5000 combined non-zero
  coefficients, and these problems carry more than 46000.
