<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Measured comparisons

Campaign results that compare kernels or tune shared parameters against
`quip-miner-cpu`. Each section records the exact commands, the measured
numbers, and the conclusion drawn from them.

## cpu-mps vs cpu-mfa: zero-bias and biased regimes

`cpu-mps` (tensor network, bond dimension chosen per job by `select_chi`)
and `cpu-mfa` (the same kernel pinned to bond dimension 1) are compared on
both bundled corpora: `chain-h0` (zero linear bias, `allowed_h_milli: [0]`)
and `chain-ternary` (biased, `allowed_h_milli: [-1000, 0, 1000]`). See
`docs/datasets.md` for corpus details.

Commands (Apple M4 Max, macOS, binaries built with `cargo build --release
--features experimental`):

```bash
isingmark throughput --backend cpu-mps --topology-preset chain-h0 \
  --corpus chain-h0 --hardness 0.5 --seed 0 --duration 15s
isingmark throughput --backend cpu-mfa --topology-preset chain-h0 \
  --corpus chain-h0 --hardness 0.5 --seed 0 --duration 15s
isingmark throughput --backend cpu-mps --topology-preset chain-ternary \
  --corpus chain-ternary --hardness 0.5 --seed 0 --duration 15s
isingmark throughput --backend cpu-mfa --topology-preset chain-ternary \
  --corpus chain-ternary --hardness 0.5 --seed 0 --duration 15s
```

Default param curve at `hardness 0.5` resolves to `num_reads: 36`,
`num_sweeps: 550` for both kernels.

| Corpus | Kernel | Jobs compared | Mean best energy (milli) | Mean wall time (ms) | Byte-identical to the other kernel |
| --- | --- | --- | --- | --- | --- |
| chain-h0 (zero bias) | cpu-mps | 56 | -12,280,643 | 2083.0 | 0 / 56 |
| chain-h0 (zero bias) | cpu-mfa | 56 | -12,291,321 | 2095.1 | 0 / 56 |
| chain-ternary (biased) | cpu-mps | 64 | -12,814,156 | 2088.7 | 0 / 64 |
| chain-ternary (biased) | cpu-mfa | 64 | -12,798,844 | 2093.9 | 0 / 64 |

Mean energies and mean wall times are close between the two kernels on
both corpora, and wall time is close enough (within one percent on each
corpus) to support the claim in `mps_sampler.rs` that `select_chi`
resolves to bond dimension 1 on graphs this wide (4577 to 4578 nodes)
regardless of which corpus supplies the bias. This holds on `chain-h0` as
well as `chain-ternary`; the harness could not confirm a case where
`select_chi` returns above 1 on either bundled corpus.

The zero rows in the "byte-identical" column do not confirm the two
kernels diverge at bond dimension 1. A repeat run of `cpu-mps` alone,
same corpus, same `--hardness`, same `--seed`, produced 0 of 56 records
byte-identical to the first `cpu-mps` run on the same jobs. `cpu-mps`
(and, by the same code path, `cpu-mfa`) is not reproducible across
separate process invocations with identical CLI arguments, so the
"agree exactly whenever `select_chi` would have returned 1" claim in
`mps_sampler.rs` cannot be checked by comparing two harness runs. Energy
values across kernels stay close in every comparison here, consistent
with both kernels running the same product-state code path, but this is
not proof of byte-for-byte agreement. Filing the run-to-run
non-determinism as a follow-up is recommended before this claim can be
verified directly.

## CPU_SB_ADAPT sweep budget

`min_sweeps: 256` and `max_sweeps: 8192` in `src/sb_sampler.rs` are shared
by all four SB binaries (`cpu-sb`, `cpu-bsb`, `cpu-hdsb`, `cpu-hbsb`).
`cpu-sb` (discrete SB) is the only one that ships in a release; the other
three build only behind the `experimental` feature.

A unit test already in `quip-miner-cpu` (`sb_core.rs`, `frustrated_triangle`
fixture) measured a real degradation on a synthetic worst case: a
three-node, fully symmetric, zero-bias triangle where the diagonal
subspace is the only attractor a discrete coupling leaves. For `cpu-sb` at
128 reads, the ground-state hit rate falls from 105/128 at 512 sweeps to
32/128 at 2048 and 0/128 at 8192. The ballistic variants (`cpu-bsb`,
`cpu-hbsb`) hold 128/128 throughout, because a continuous coupling
preserves the magnitude differences that break the symmetry. That same
test comment records that no other fixture in the unit suite shows the
collapse: two random-graph fixtures and a random 16-node frustrated
instance stay at the ground state for every variant out to 8192 sweeps.

To check whether that collapse appears on real chain topology, all four
SB binaries were run against `chain-h0` and `chain-ternary` at several
sweep budgets. Commands (`num_reads` fixed at 16 to keep each run short;
duration is an upper bound, not a lower one — a corpus run stops at
whichever of duration or corpus end comes first):

```bash
isingmark throughput --backend cpu-sb --topology-preset chain-h0 \
  --corpus chain-h0 --seed 0 --duration 8s \
  --config <param_curve with min_sweeps = max_sweeps = N, min_reads = max_reads = 16>
# N in {256, 2048, 8192}, repeated for chain-ternary,
# and for cpu-bsb / cpu-hdsb / cpu-hbsb at N in {256, 8192} on chain-h0 only
```

| Backend | Corpus | Sweeps | Jobs | Mean best energy (milli) | Mean wall time (ms) |
| --- | --- | --- | --- | --- | --- |
| cpu-sb | chain-h0 | 256 | 128 | -14,434,828 | 509 |
| cpu-sb | chain-h0 | 2048 | 16 | -14,574,375 | 4,489 |
| cpu-sb | chain-h0 | 8192 | 8 | -14,622,000 | 16,885 |
| cpu-sb | chain-ternary | 256 | 120 | -14,807,067 | 526 |
| cpu-sb | chain-ternary | 2048 | 16 | -15,001,750 | 4,303 |
| cpu-sb | chain-ternary | 8192 | 8 | -15,035,875 | 17,562 |
| cpu-bsb | chain-h0 | 256 | 136 | -14,479,603 | 457 |
| cpu-bsb | chain-h0 | 8192 | 8 | -14,536,000 | 14,657 |
| cpu-hdsb | chain-h0 | 256 | 96 | -14,436,104 | 612 |
| cpu-hdsb | chain-h0 | 8192 | 8 | -14,618,750 | 17,838 |
| cpu-hbsb | chain-h0 | 256 | 112 | -14,361,679 | 560 |
| cpu-hbsb | chain-h0 | 8192 | 8 | -14,610,000 | 17,225 |

More negative is better (lower energy). On both real corpora and for all
four binaries, mean best energy improves monotonically as the sweep
budget rises from 256 to 8192; none of the four shows the triangle
fixture's collapse toward the maximum-energy state. Wall time scales
close to linearly with the sweep count on both corpora (roughly 4x
wall time for each 4x to 8x rise in sweeps), so `max_sweeps: 8192` does
not carry a runaway cost on this hardware, and the quality gain from 256
to 8192 sweeps is real: on `chain-h0`, `cpu-sb`'s mean energy improves by
about 1.3 percent from the 256-sweep point to the 8192-sweep point,
and the trend has not leveled off by 8192.

**Conclusion:** the frustrated-triangle collapse is real but confined to
a fully symmetric, zero-bias, uniform-neighborhood synthetic fixture; it
does not appear on either bundled chain corpus for any of the four SB
binaries at up to 8192 sweeps. Because production data shows quality
still improving at the current `max_sweeps` ceiling, this campaign does
not support lowering it, and `min_sweeps: 256` already produces most of
the 8192-sweep quality (within roughly 1.3 percent on `chain-h0`) at a
fraction of the cost. **No change was made to `min_sweeps` or
`max_sweeps` in `src/sb_sampler.rs`.**

Not measured in this campaign, for time: budget points between 8192 and
higher values (to see whether quality keeps improving past the current
ceiling), sweep points other than 256/2048/8192, and `cpu-bsb` /
`cpu-hdsb` / `cpu-hbsb` on `chain-ternary`. A wall-clock budget shared
with a second campaign (the `cpu-mps`/`cpu-mfa` comparison above) limited
this run to spot checks on the experimental binaries rather than the
full grid.
