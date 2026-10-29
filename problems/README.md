<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Problem families

Each folder in this directory describes one problem family of the QPU suite discovery run. The folder holds the construction, the variants, the measured results, and the provenance of that family.

## Index

The Code column holds the candidate code of the family. A dash means the family has no code. The Corpus files column counts cells that have a frozen-model example in `corpora/`. Lowest cell tier is the lowest value of the cell `tier` field among the family cells. Tier 1 and tier 2 mean no classical run beats the QPU on the criteria of that field. Tier 3 and tier 4 mean a classical run does. Each family README defines the tiers in full.

| Family | Code | Cells | Corpus files | Status | Lowest cell tier |
| --- | --- | --- | --- | --- | --- |
| [`ice-three-level`](ice-three-level/README.md) | F14 | 1 | 1 of 1 | Closed | 3 |
| [`ice-planted`](ice-planted/README.md) | F13 | 1 | 1 of 1 | Closed | 4 |
| [`cbfm`](cbfm/README.md) | F17 | 4 | 3 of 4 | Closed | 4 |
| [`strong-net`](strong-net/README.md) | F2 | 3 | 3 of 3 | Closed | 3 |
| [`glass-3d`](glass-3d/README.md) | F15,F16 | 2 | 2 of 2 | Closed | 3 |
| [`ice-glass`](ice-glass/README.md) | F11 | 3 | 3 of 3 | Closed | 4 |
| [`tri-match`](tri-match/README.md) | F1,F9 | 3 | 3 of 3 | Closed | 3 |
| [`dimer`](dimer/README.md) | F6,F10 | 4 | 4 of 4 | Closed | 4 |
| [`twisted-ice`](twisted-ice/README.md) | F12 | 25 | 25 of 25 | Closed | 1 |
| [`wsc`](wsc/README.md) | F7 | 1 | 1 of 1 | Closed | 2 |
| [`dcl`](dcl/README.md) | F5,F5b | 3 | 3 of 3 | Closed | 1 |
| [`fcl-planted`](fcl-planted/README.md) | F4 | 2 | 2 of 2 | Closed | 2 |
| [`strip-glass`](strip-glass/README.md) | F19 | 1 | 1 of 1 | Closed | 4 |
| [`frust-dense`](frust-dense/README.md) | F8 | 2 | 2 of 2 | Closed | 3 |
| [`kagome-net`](kagome-net/README.md) | F18 | 1 | 1 of 1 | Closed | 4 |
| [`pair-loop`](pair-loop/README.md) | F3 | 1 | 1 of 1 | Closed | 3 |
| [`ice-defect`](ice-defect/README.md) | F20-F23 | 6 | 6 of 6 | Closed | 3 |
| [`ice-fields`](ice-fields/README.md) | F24 | 2 | 2 of 2 | Closed | 3 |
| [`ice-overlap`](ice-overlap/README.md) | F22 | 4 | 4 of 4 | Closed | 3 |
| [`parity-checks`](parity-checks/README.md) | - | 3 | 3 of 3 | Closed | 3 |
| [`dimer-interacting`](dimer-interacting/README.md) | - | 2 | 2 of 2 | Closed | 3 |
| [`reductions`](reductions/README.md) | - | 6 | 6 of 6 | Closed | 1 |
| [`ice-random-k4`](ice-random-k4/README.md) | - | 1 | 1 of 1 | Closed | 3 |
| [`charged-ice`](charged-ice/README.md) | - | 1 | 1 of 1 | Closed | 3 |
| [`commercial`](commercial/README.md) | - | 2 | 0 of 2 | Closed | 3 |
| [`ice-pairs-constraint`](ice-pairs-constraint/README.md) | - | 1 | 1 of 1 | Closed | 4 |
| [`ice-stack`](ice-stack/README.md) | - | 2 | 2 of 2 | Closed | 3 |

The index lists 27 families and 87 cells. 84 cells have a frozen-model example in `corpora/`. The other cells have none. `cbfm-p-f17` has no frozen model. `sports-break` and `paint-shop` use their own small graphs, not the chain-h0 topology.

## Folder layout

Each family folder holds these items:

- `README.md` describes the family. It has the construction, the variants, the generator and postprocess notes, the results, the limits, and the provenance.
- `params/` holds one generation parameter file per cell. Not every family has this folder.
- `sources/` holds byte-identical copies of the research files that only this family uses.

Two items sit beside the family folders:

- `shared/sources/` holds copies of research files that more than one family uses.
- `sources.json` is the manifest. It lists every copied file with its path, size, and blake3 digest.

Never edit a copied source in place. Copy a new snapshot from the research workspace and update the manifest entry.

## How to verify

Run these commands:

```bash
cargo test --test campaign_source_inventory
cargo test --test corpora_instances
```

The first test checks that every file in `sources.json` exists and matches its recorded size and digest. It also checks that every file under `problems/*/sources/` is listed. The second test loads every example corpus in `corpora/` and checks it against its topology.

See `reports/import-map.md` for the origin of each copied file and for the function to ticket map.
