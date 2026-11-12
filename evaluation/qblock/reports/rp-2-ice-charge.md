# Reproduction of the charged-ice cell, classical arm

QUI-2232 (RP-2), under the RP-0 protocol (QUI-2230). Runs of 2026-11-12.

The control verdict is **not reproduced**. The Tier label and the energy gap reproduce. The time-to-target check fails because `quip-cpu-sa` with its default schedule reaches the archived target on only 23 of 100 models. The fresh verdict is **incomplete: waiting on RP-6**.

## Result

| Check | Archived | Reproduced | Pass |
|---|---|---|---|
| Tier, rule v3 | 3 | 3 | yes |
| Gap, mean (sd) | +1.950 (0.966) units, pass within 1.932 | +2.010 (0.854) units, 100 models | yes |
| Time to target | CPU 1.897 s | CPU reaches the target on 23 of 100 models | no |

The archived row takes its time from the CPU, because the archived `glasssa` run reached `qpu80_pp` in 1.897 core-seconds per model. The production `quip-cpu-sa` uses its default schedule, not the tuned `glasssa` schedule. It reached `qpu80_pp` on 23 of 100 models, so the check has no time to compare. Its median wall of 4.477 s per model also exceeds the pass limit of 3.793 s, which is two times the archived value. The check would fail on time even if the arm had reached the target. Two differences of basis apply. The archived value is the median over 30 models of core-seconds, and the reproduced value is the median over 100 models of the wall to the first batch receipt. The runner starts that clock before it encodes and releases the job.

The CUDA arm reached `qpu80_pp` on all 100 models at a median of 23.274 s per model. The archived CUDA time is 4.564 s per model, but that value is the batch wall divided by 33 or 34 jobs that shared one card. This run gives the card to one job at a time, so the two CUDA times are not directly comparable.

## Control arm on the archived models

| Device | Policy | Models | QPU / tie / classical | Gap mean (sd) | Median energy | Median wall s |
|---|---|---:|---|---|---:|---:|
| CPU | `rp2-cpu-sa-r1-s72155` | 100 | 77 / 0 / 23 | -0.867 (1.319) | -1527.502 | 4.477 |
| CUDA | `rp2-cuda-msa-descent-r64-s65536` | 100 | 0 / 0 / 100 | +2.010 (0.854) | -1530.535 | 23.274 |

A positive gap means that the classical energy is below `qpu80_pp`. The control instances are the 100 archived models, written as `ising-fixed` instances with the target `(qpu80_pp + 1) / 1000`.

## Fresh cohort

| Device | Policy | Models | With state | Median energy | Mean energy | Range | Median wall s |
|---|---|---:|---:|---:|---:|---|---:|
| CPU | `rp2-cpu-sa-r1-s72155` | 30 | 30 | -1531.549 | -1529.993 | -1552.704 to -1510.414 | 4.512 |
| CUDA | `rp2-cuda-msa-descent-r64-s65536` | 30 | 30 | -1534.389 | -1532.512 | -1553.894 to -1512.654 | 23.284 |

The 30 fresh models are new `quip/charged-ice` revision 1 instances of the `ice-charge` recipe. Their target is a placeholder until RP-6 supplies `qpu80_pp`, so every trial ends at its evaluation limit, and the table reports energies only. Every trial kept its final state.

## Setup

| Item | Value |
|---|---|
| Frozen comparison | `evaluation/qblock/configs/reproduction/ice-charge.json`, frozen 2026-11-12 |
| Policies | `evaluation/qblock/configs/reproduction/ice-charge.policies.json` |
| CPU miner | `quip-cpu-sa` 0.3.6-rc5, 1 read, 72,155 sweeps, default schedule, and default seed |
| CUDA miner | `quip-cuda-msa-descent` 0.3.6-rc3, 64 reads, 65,536 sweeps, then host greedy and loop descent |
| Host | AMD Ryzen 9 5950X, NVIDIA RTX A4000 (GPU 0), driver 615.71.10 |
| Order | CPU control, CPU fresh, CUDA control, CUDA fresh, one trial at a time |
| Wall window | 1,200 s per trial |
| Machine output | `evaluation/qblock/reports/rp-2-ice-charge.comparison.json` |

The CUDA miner recovers the K4 tiles from the couplings of the control instances, because those instances carry no tile metadata. The fresh instances carry their tiles in the `quip/charged-ice` metadata.

## Change to the frozen comparison

The comparison was first frozen on 2026-11-10. That freeze ran the CUDA arm on `quip-cuda-sa` 0.3.5 with 64 reads and 65,536 sweeps, and no descent. On this host, `quip-cuda-sa` needs about 680 s per job at that budget. The miner stopped each job at its 600 s stall floor and answered `OVERLOADED`. A second try used `quip-cuda-msa` 0.3.5, which has no descent step. Then CU-7 (QUI-2242) shipped `quip-cuda-msa-descent` in `quip-solver-cuda` 0.3.6-rc3, which runs the archived pipeline: the msa kernel, then greedy and loop descent. The second freeze, on 2026-11-12, uses that miner. It came before the runs in this report.

The earlier runs are in `/zfspool/quip/isingmark-runs/rp-2/superseded/`:

| Directory | Miner | Result |
|---|---|---|
| `control-cpu-sa/` | `quip-cpu-sa` 0.3.5 | 100 trials, 31 at or below `qpu80_pp` |
| `fresh-cpu-sa/` | `quip-cpu-sa` 0.3.5 | 30 trials, all at the evaluation limit |
| `control-cuda/` | `quip-cuda-sa` 0.3.5 | stopped after 3 trials, all rejected as `OVERLOADED` |
| `control-cuda-msa/` | `quip-cuda-msa` 0.3.5 | stopped after 18 trials, all accepted |

The CPU policy did not change between the two freezes. The CPU control reached the target on 31 of 100 models with 0.3.5. With 0.3.6-rc5 it reached 23 of 100. The CPU result depends on the miner version.

## Stored data

The campaign directories hold every event, state, and trace. They are on this host, under `/zfspool/quip/isingmark-runs/rp-2/`:

* `control-cpu-sa/`, `control-cuda-msa-descent/`: the control arm.
* `fresh-cpu-sa/`, `fresh-cuda-msa-descent/`: the fresh cohort.
* `report/`: the output of `isingmark campaign reproduce-report`.
* `run-rc.sh`: the script that ran the four campaigns.

## Reproduce

Put the two miner versions of the Setup table on `PATH`. `tools/fetch-miners.sh` fetches the newest release of each repository, which can be newer. Run these commands from the repository root. `RUNS` is a new directory, and `POLICIES` is `evaluation/qblock/configs/reproduction/ice-charge.policies.json`.

```sh
# Control inputs: the 100 archived models, from the read-only research root.
python -m isingmark_structured.reproduction --cell ice-charge \
  --policies POLICIES --out RUNS/control-inputs
# Fresh inputs: 30 new charged-ice models.
isingmark campaign cohort --recipe ice-charge --count 30 \
  --topology fixtures/suite-live-2026-09-21.spec.json --max-energy -1000000 \
  --policies POLICIES --out RUNS/fresh-inputs
# The four campaigns, one at a time.
for backend in cpu-sa cuda-msa-descent; do
  for cohort in control fresh; do
    isingmark campaign --manifest RUNS/$cohort-inputs/manifest.json \
      --backend $backend --output RUNS/$cohort-$backend
  done
done
isingmark campaign reproduce-report \
  --frozen evaluation/qblock/configs/reproduction/ice-charge.json \
  --control RUNS/control-cpu-sa --control RUNS/control-cuda-msa-descent \
  --fresh RUNS/fresh-cpu-sa --fresh RUNS/fresh-cuda-msa-descent \
  --out RUNS/report
```

## Open items

* A CPU miner with the `glasssa` schedule would let the time-to-target check compare against the archived CPU time.
* RP-6 supplies `qpu80_pp` for the fresh cohort. The fresh verdict waits on it.
