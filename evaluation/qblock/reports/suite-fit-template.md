# Suite-fit report for `PAIR_ID`

This template defines one A/B report. Replace each uppercase placeholder with a value from the run. Do not add a number that no file in the run supplies.

## Sources

| Field | Value |
| --- | --- |
| Pair | `PAIR_ID` |
| Isingmark revision | `GIT_REVISION` |
| A source | `archived` from `evaluation/qblock/archive-index.json`, or `campaign` from `RUN_DIR_A` |
| B source | `campaign` from `RUN_DIR_B` |
| Archive source hash | `SOURCE_BLAKE3` |
| Analysis design | wall grid `WALL_GRID`, device grid `DEVICE_GRID`, censoring `CENSORING`, resamples `RESAMPLES`, seed `SEED` |
| Group map | `GROUPS_FILE`, or "each instance is its own group" |

## Comparison table

`ab-comparison.csv` holds one row per arm. The table lists the column groups.

| Columns | Meaning |
| --- | --- |
| `pair_id`, `side`, `source` | The pair, the arm, and `campaign` or `archived` |
| `family`, `family_revision`, `parameter_digest`, `policy_id` | The identity of the arm |
| `size_spins`, `size_status` | The arm size and `matched` or `unmatched_size` |
| `target_rule`, `panel`, `reference` | The success rule, the `native` or `common` target panel, and the energy reference |
| `models`, `groups`, `trials`, `unresolved`, `state_coverage` | Instances or reference models, independent groups, trials, unresolved device trials, and the share of trials with every state captured in every stage |
| `wall_f_T`, `_lower`, `_upper` | Wall success F at T seconds with its pointwise 95 percent band |
| `device_g_30`, `_lower`, `_upper` | Quantum processing unit (QPU) device success G at 30 device seconds with its band |
| `device_timing`, `device_s_lower`, `device_s_upper` | The worst device timing status and the total device seconds |
| `host_compute_s`, `evaluations` | The host compute seconds and evaluation count over all trials |
| `comparison` | `estimated`, or the reason the pair has no A−B difference |
| `tier_rule_version`, `historical_tier`, `tier_beat_wall`, `tier_speed_wall` | The historical Tier label of an archived arm and the wall basis of its beat run and its speed run |

An empty field means the run did not measure that value. It never means zero.

## Rules for reading the table

1. An archived arm has no success curve. Its row carries the `qpu80_pp` reference, the model count, and the historical Tier label.
2. The pairs `p-charge`, `d-charge`, `f-h500`, `f-h1000`, and `f-routing` stay `unmatched_size` until a matched A cohort clears the size gate. Do not report an adaptation effect for them.
3. An A−B difference exists only when both arms hold campaign trials, the sizes match, and the target panels agree. It uses only the groups with trials in both arms, and it stops at the shorter observation window.
4. Native-target and common-target panels of a synthetic pair are separate rows. Do not combine them.
5. Related templates, cost-scaled F requests, shared A runs, and sessions form one group each. A group counts as one observation and gives its instances equal weight.
6. A missing state is a failure. A trial without device timing stays unresolved and widens the device band.
7. The Tier label follows Tier rule version 3 and needs at least 30 common models. Use the label only as a historical comparison, never as a probability, a guide, or proof of physical validity.
8. Keep the `qpu80_pp` reference and each wall timing basis in separate columns. A modeled wall never decides a Tier label. The `tier_beat_wall` and `tier_speed_wall` columns show `scaled` for a scaled wall.

## Paired differences

| Clock | Grid seconds | Estimate | Lower | Upper | Common groups | A only | B only |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Wall | `T` | `DIFF` | `LOWER` | `UPPER` | `GROUPS` | `A_ONLY` | `B_ONLY` |
| Device | `T` | `DIFF` | `LOWER` | `UPPER` | `GROUPS` | `A_ONLY` | `B_ONLY` |

Write "not comparable: `REASON`" instead of this table when `comparison` is not `estimated`.

## Assumptions

Copy the assumption text of every curve and difference here, word for word.

## Skipped checks

List each check that did not run and the reason.
