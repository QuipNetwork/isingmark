// SPDX-License-Identifier: AGPL-3.0-or-later
//! Hardness-to-job-parameter mapping.
//!
//! Maps a normalized `hardness` value to concrete per-job sample parameters
//! by linear interpolation between a backend's min and max bounds.
//! `hardness` is clamped to `[0.0, 1.0]`; `NaN` is treated as `0.0` (the
//! safest, least demanding setting) since it cannot be compared against the
//! bounds. `hardness = 0.0` lands exactly on the curve minimum and
//! `hardness = 1.0` lands exactly on the curve maximum.

/// Backend execution model. `Sweeping` backends (CPU SA/Gibbs, CUDA, Metal,
/// BEIT) are driven by `num_reads`/`num_sweeps`; `Annealing` backends (D-Wave,
/// cuOpt) are driven by `num_reads`/`anneal_time_us`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    /// CPU SA/Gibbs, CUDA, Metal, BEIT: reads + sweeps.
    Sweeping,
    /// D-Wave, cuOpt: reads + anneal time.
    Annealing,
}

/// Per-job sample parameters resolved from a [`ParamCurve`]. A field value of
/// `0` means "unset": the miner falls back to the `SetTarget` control-plane
/// default or its own adapt logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobParams {
    /// Independent reads (samples) to take. Always set.
    pub num_reads: u32,
    /// Annealing sweeps per read. Set only for [`BackendKind::Sweeping`].
    pub num_sweeps: u32,
    /// Anneal time in microseconds. Set only for [`BackendKind::Annealing`].
    pub anneal_time_us: u32,
}

/// Linear min/max bounds for each backend-specific parameter, indexed by
/// `hardness` in `[0.0, 1.0]`.
///
/// A bound of `0` is not "no reads" or "no sweeps" — it is the wire-level
/// UNSET sentinel, so a curve endpoint that lands on `0` stops requesting a
/// specific value there at all. For example, `min_reads: 0` means that at
/// `hardness = 0.0` the harness emits `num_reads = 0`, which the miner reads
/// as "unset" and replaces with its own `SetTarget`/adapt default instead of
/// the requested minimum — silently, with no error from `resolve` — so the
/// easiest point on the curve ends up measuring the miner's own adaptation
/// logic rather than the parameter the operator configured. Set every bound
/// that should be honored at its endpoint to a nonzero value.
///
/// `resolve` does not validate `min <= max` for any pair; keeping that
/// invariant is the caller's responsibility. A curve with `min > max`
/// interpolates the same way but runs backward (decreasing as `hardness`
/// increases), which `resolve` treats as intentional rather than an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamCurve {
    /// Reads at `hardness = 0.0`, for both backend kinds.
    pub min_reads: u32,
    /// Reads at `hardness = 1.0`, for both backend kinds.
    pub max_reads: u32,
    /// Sweeps at `hardness = 0.0`. Used only by [`BackendKind::Sweeping`].
    pub min_sweeps: u32,
    /// Sweeps at `hardness = 1.0`. Used only by [`BackendKind::Sweeping`].
    pub max_sweeps: u32,
    /// Anneal time, in microseconds, at `hardness = 0.0`. Used only by
    /// [`BackendKind::Annealing`].
    pub min_anneal_us: u32,
    /// Anneal time, in microseconds, at `hardness = 1.0`. Used only by
    /// [`BackendKind::Annealing`].
    pub max_anneal_us: u32,
}

impl ParamCurve {
    /// Resolve `hardness` into concrete [`JobParams`] for `kind`.
    #[must_use]
    pub fn resolve(&self, kind: BackendKind, hardness: f64) -> JobParams {
        let t = normalize(hardness);
        let num_reads = lerp_round(self.min_reads, self.max_reads, t);
        match kind {
            BackendKind::Sweeping => JobParams {
                num_reads,
                num_sweeps: lerp_round(self.min_sweeps, self.max_sweeps, t),
                anneal_time_us: 0,
            },
            BackendKind::Annealing => JobParams {
                num_reads,
                num_sweeps: 0,
                anneal_time_us: lerp_round(self.min_anneal_us, self.max_anneal_us, t),
            },
        }
    }
}

/// Clamp `hardness` to `[0.0, 1.0]`.
///
/// `NaN` fails both bound comparisons, so a naive clamp would leave it
/// unclamped; `f64::clamp` instead panics on `NaN`, and `panic` is a denied
/// lint here. This treats `NaN` as `0.0` (the safest, least demanding
/// setting) so `resolve` stays a total function.
fn normalize(hardness: f64) -> f64 {
    if hardness.is_nan() {
        return 0.0;
    }
    hardness.clamp(0.0, 1.0)
}

/// Linearly interpolate between `min` and `max` at `t` (already normalized
/// to `[0.0, 1.0]`), rounding to the nearest `u32`, half away from zero.
/// `t = 0.0` returns exactly `min`; `t = 1.0` returns exactly `max`.
fn lerp_round(min: u32, max: u32, t: f64) -> u32 {
    let value = f64::from(min) + (f64::from(max) - f64::from(min)) * t;
    round_u32(value)
}

/// Round a `f64` produced by [`lerp_round`] to the nearest `u32`, half away
/// from zero.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "lerp_round only ever passes values within [min, max] of u32 bounds"
)]
fn round_u32(value: f64) -> u32 {
    value.round() as u32
}

#[cfg(test)]
mod tests {
    use super::{BackendKind, ParamCurve};

    /// Bounds chosen so every min/max is nonzero and distinct across
    /// parameters (catches min/max swaps and reads/sweeps swaps), and every
    /// midpoint lands on a `.5` fraction (catches truncation instead of
    /// rounding).
    fn curve() -> ParamCurve {
        ParamCurve {
            min_reads: 10,
            max_reads: 91,
            min_sweeps: 200,
            max_sweeps: 999,
            min_anneal_us: 5,
            max_anneal_us: 54,
        }
    }

    #[test]
    fn sweeping_at_zero_hits_min_bounds_exactly() {
        let params = curve().resolve(BackendKind::Sweeping, 0.0);
        assert_eq!(params.num_reads, 10);
        assert_eq!(params.num_sweeps, 200);
        assert_eq!(params.anneal_time_us, 0);
    }

    #[test]
    fn sweeping_at_one_hits_max_bounds_exactly() {
        let params = curve().resolve(BackendKind::Sweeping, 1.0);
        assert_eq!(params.num_reads, 91);
        assert_eq!(params.num_sweeps, 999);
        assert_eq!(params.anneal_time_us, 0);
    }

    #[test]
    fn sweeping_at_half_rounds_midpoint_to_nearest() {
        let params = curve().resolve(BackendKind::Sweeping, 0.5);
        // (10 + 91) / 2 = 50.5 -> 51 (round half away from zero, not truncate).
        assert_eq!(params.num_reads, 51);
        // (200 + 999) / 2 = 599.5 -> 600.
        assert_eq!(params.num_sweeps, 600);
        assert_eq!(params.anneal_time_us, 0);
    }

    #[test]
    fn sweeping_never_sets_anneal_time() {
        for hardness in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let params = curve().resolve(BackendKind::Sweeping, hardness);
            assert_eq!(params.anneal_time_us, 0);
        }
    }

    #[test]
    fn annealing_at_zero_hits_min_bounds_exactly() {
        let params = curve().resolve(BackendKind::Annealing, 0.0);
        assert_eq!(params.num_reads, 10);
        assert_eq!(params.anneal_time_us, 5);
        assert_eq!(params.num_sweeps, 0);
    }

    #[test]
    fn annealing_at_one_hits_max_bounds_exactly() {
        let params = curve().resolve(BackendKind::Annealing, 1.0);
        assert_eq!(params.num_reads, 91);
        assert_eq!(params.anneal_time_us, 54);
        assert_eq!(params.num_sweeps, 0);
    }

    #[test]
    fn annealing_at_half_rounds_midpoint_to_nearest() {
        let params = curve().resolve(BackendKind::Annealing, 0.5);
        assert_eq!(params.num_reads, 51);
        // (5 + 54) / 2 = 29.5 -> 30.
        assert_eq!(params.anneal_time_us, 30);
        assert_eq!(params.num_sweeps, 0);
    }

    #[test]
    fn annealing_never_sets_num_sweeps() {
        for hardness in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let params = curve().resolve(BackendKind::Annealing, hardness);
            assert_eq!(params.num_sweeps, 0);
        }
    }

    #[test]
    fn hardness_below_zero_clamps_to_min() {
        let params = curve().resolve(BackendKind::Sweeping, -5.0);
        assert_eq!(params.num_reads, 10);
        assert_eq!(params.num_sweeps, 200);
    }

    #[test]
    fn hardness_above_one_clamps_to_max() {
        let params = curve().resolve(BackendKind::Sweeping, 5.0);
        assert_eq!(params.num_reads, 91);
        assert_eq!(params.num_sweeps, 999);
    }

    #[test]
    fn nan_hardness_is_treated_as_zero() {
        let params = curve().resolve(BackendKind::Sweeping, f64::NAN);
        assert_eq!(params.num_reads, 10);
        assert_eq!(params.num_sweeps, 200);
    }
}
