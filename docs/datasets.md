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

A fixture holds no coefficient arrays. The arrays stay in the research
archive, in the model file that `archive_origin` names. The fixture records the
nonce, the generator recipe, the BLAKE3 hash of the archived file, two digests
of the model, and the packed witness spins. A later port of a family can
rebuild the model from the nonce and compare it with these digests.

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
      "node_order": "physical",
      "n_nodes": 4575,
      "n_edges": 41474,
      "topology_order_digest": "<64 hex>",
      "model_digest": "<64 hex>",
      "offset_milli": "0",
      "qubo_offset_milli": null,
      "family_id": null,
      "descriptor_digest": null,
      "params_hex": null,
      "postprocess_policy": {
        "family": "ice-tiles",
        "method": "anneal:b0=2",
        "budget_s": 0.25,
        "sources": [{ "path": "<research-root-relative file>", "blake3": "<64 hex>" }]
      },
      "witnesses": [
        {
          "name": "qpu-best",
          "spins_packed": "<hex>",
          "claimed_energy_milli": "-1520448",
          "origin": { "path": "<research-root-relative file>", "blake3": "<64 hex>", "array": "spins", "index": 0 }
        },
        { "name": "greedy", "missing": "states not stored, searched <research-root-relative path>" }
      ]
    }
  ]
}
```

### Witnesses

Each instance lists up to three witnesses in this order. Each one is an
archived observation that the research showed before, so the fixture does not
label any witness as a new result.

- `qpu-best` is the raw QPU read with the lowest energy, from the `spins` array
  of the QPU file. Its energy equals `qpu80_best`.
- `greedy` is the state that `qpu80_greedy` came from. Every cell stores the
  `gnat_c` reads, and the witness takes the lowest one. Ice cells also store
  `loop_strict` reads. The witness takes the `loop_strict` state of the read with
  the lowest raw energy when that state is at least as low as the best `gnat_c`
  state. Its energy equals `qpu80_greedy`.
- `pp-final` is the final postprocessed state. Its energy equals `pp_best`.

The exporter recomputes every energy from the archived model arrays and stops on
any difference. A witness whose state file is not in the archive has a
`missing` reason that names the paths searched. The fixture holds no timing
fields.

`origin` ties a spins witness to the archive file it came from, in the same way
that `archive_origin` ties the model. `path` is relative to the research root.
`blake3` is the hash of that file. `array` is the key inside an NPZ file, or
null for a single `.npy` state. `index` is the read in that array, or null for a
single state.

`postprocess_policy.sources` lists the `*.py` files that sit directly in the
optimizer directory of the policy `family`, sorted by path. Each entry has a
path relative to the research root and the hash of the file. The repository does
not hold copies of these files.

### Digests

Both digests use BLAKE3 in key-derivation mode. The canonical instance layout
of quip-solver-core (FAM-30) is a 97-byte head followed by a body. The body holds
the `u32` little-endian node count, the node ids, the `u32` edge count, the
edges as dense positions, the `h` values as `i32`, the `j` values as `i32`,
`offset_milli` as `i64`, and the metadata region. These fixtures use the empty metadata region, the two bytes `00 00`.

- `topology_order_digest` is the upstream topology order digest (FAM-19). It
  does not read the whole layout. Its context is
  `quip family topology order v1`. It covers the node count, the node ids, the
  edge count, and each edge as two native node ids, in instance order.
- `model_digest` is an IsingMark rule, not an upstream digest. Its context is
  `isingmark archived model v1`. It reads the FAM-30 body only, including the
  metadata region. The research cells have no family descriptor yet, so the
  upstream instance digest (FAM-18) does not apply to them.

`spins_packed` uses the wire packing of the protocol. Spin `i` is bit `i % 8`
of byte `i / 8`, low bit first. Bit 1 means `+1` and bit 0 means
`-1`. The field has exactly `ceil(n_nodes / 8)` bytes, and the padding bits are 0.

The pinning tests keep the two encoders honest. They build one small
instance, with 3 nodes, in Rust through `ising-generators` and in Python
through `archive.py`. Both must give the same pinned hex constants. A change to
either encoder fails a test.

### Validation rules

The validator reports every violation it finds in a file, and each message
names the file and the instance. Two kinds of fault differ. A parse failure
(an unknown key, a missing field, or a wrong type) stops the check, so the
validator reports it alone for that file. A fault in the file header or in
`sources` names the file only. The validator does not check the digests against
the archive, because the archive is not in the repository. The rules are:

- `schema_revision` is `1`, `provenance` is `archived-research`, and `about`
  is not empty. The validator rejects unknown keys at every level.
- Each `sources[].path` is a safe relative path with `/` separators and no
  empty, `.`, or `..` part. The file exists in the repository, and its BLAKE3
  hash equals `blake3`.
- Instance names are unique within the file.
- `nonce` and `archive_blake3` are 64 lowercase hex characters.
  `archive_origin` is a safe relative path. The validator does not check that
  it exists. It checks the form of `archive_blake3` only.
- `source_recipe.source` names a source of kind `generator` in
  `problems/sources.json`, and `source_recipe.name` is not empty. `cell` is
  not empty.
- `node_order` is `physical` or `ascending`.
- `n_nodes` and `n_edges` are positive integers.
- `topology_order_digest` and `model_digest` are 64 lowercase hex characters.
- `offset_milli`, `qubo_offset_milli` when it is not null, and
  `claimed_energy_milli` are decimal strings that parse as 64-bit integers. A
  JSON number in any of these fields fails validation.
- `family_id`, `descriptor_digest`, and `params_hex` are each null or valid:
  a non-empty string, 64 lowercase hex characters, and even-length lowercase
  hex.
- `postprocess_policy` has a non-empty `family` and `method`, and `budget_s`
  is zero or more. `sources` is not empty. Each entry has a safe relative `path`
  and a `blake3` of 64 lowercase hex characters. The entries are sorted by path,
  and no path repeats.
- A witness has either `spins_packed` with `claimed_energy_milli`, or a
  `missing` reason that is not empty. It never has both and never has neither.
  Witness names are unique within the instance.
- A witness with `spins_packed` has an `origin`, and a `missing` witness has
  none. In `origin`, `path` is a safe relative path, `blake3` is 64 lowercase
  hex characters, `array` is null or a non-empty string, and `index` is null or
  a non-negative integer. Both `array` and `index` must be present.
- `spins_packed` is lowercase hex of exactly `ceil(n_nodes / 8)` bytes, and its
  padding bits are 0.

The export command writes the fixtures of seven cells from the research
archive. It reads the first two rows of each cell and rejects a model that does
not fit the format. It records the two digests and the policy sources. It packs
the three witnesses when the archive stores their states. It requires the
energy of each state, computed from the archived model with no offset, to equal
the recorded value: `qpu80_best`, `qpu80_greedy`, or `pp_best`.

```bash
python -m isingmark_structured.archive --research-root <path>
```

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

## Archive index

`evaluation/qblock/archive-index.json` lists the archived research rows of seven cells. The cells
are `ice-charge`, `ice-fields-h500`, `ice-fields-h1000`, `two-commodity-routing`,
`ice-twisted-w62-dimer1200`, `ice-twisted-w62-dimer1350`, and `ice-twisted-w45-dimer1200`. Each cell
has the archive `family`, `method`, and `budget_s`, and 100 rows in archive order.
The `family` is the archive value, not a `problems/` slug.

A row holds the nonce, the model path relative to `research_root`, the blake3 digest of the model
file, and the archived energies `qpu80_best`, `qpu80_greedy`, and `qpu80_pp`. Energies are decimal
strings. `pp_cpu_s` is a JSON number. The top level names `research_root` once and records the
digest of the source manifest `new-pow/targets-pp-100.json`.

Rebuild the index from a read-only research root:

```bash
cd tools/structured
python -m isingmark_structured.archive_index --root /path/to/regimes
```

The test `tests/campaign_archive_index.rs` checks the index shape. It also checks that every
archived fixture instance appears in the index with the same model path and digest. Last, it
checks that no file under `src/` names a research path.
