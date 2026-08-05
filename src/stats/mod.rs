// SPDX-License-Identifier: AGPL-3.0-or-later
//! Aggregate statistics and two-sample distribution comparison for
//! benchmark energy/metric samples.
//!
//! Every public function here is pure computation: no I/O, no protocol
//! types. All of them are total over their inputs — bad input (an empty
//! slice, an out-of-range `q`, a non-finite value) produces `None` rather
//! than a panic or a silently wrong number, per this crate's fail-fast
//! convention (see `contains_non_finite` below for why non-finite values
//! are rejected rather than filtered or propagated).

pub mod aggregate;
pub mod ks;

pub use aggregate::{describe, percentile, Stats};
pub use ks::{ks_distance, ks_two_sample_p};

/// `true` if `values` contains a `NaN`, `+inf`, or `-inf`.
///
/// Every function in this module calls this before doing arithmetic and
/// returns `None` if it is `true`, rather than filtering the offending
/// values out or letting them propagate. Two reasons:
///
/// 1. Filtering silently changes the sample size the caller thinks they
///    passed in, which would make a reported `Stats::mean` or
///    `ks_distance` quietly answer a different question than the one
///    asked.
/// 2. Letting a `NaN` propagate (e.g. into `f64::total_cmp`-based
///    sorting, which places `NaN` at the sorted extremes rather than
///    erroring) produces a *plausible-looking* percentile or KS distance
///    that is simply wrong — the exact failure mode this project is
///    built to guard benchmark numbers against.
///
/// Extreme-but-finite values (`f64::MAX`, the `1 << 62` overflow sentinel
/// used upstream in `quip_protocol::scoring`) are realistic benchmark
/// data and are *not* rejected here — only actual `NaN`/infinite values
/// are.
pub(crate) fn contains_non_finite(values: &[f64]) -> bool {
    values.iter().any(|v| !v.is_finite())
}
