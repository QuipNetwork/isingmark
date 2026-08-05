// SPDX-License-Identifier: AGPL-3.0-or-later
//! Human-readable report rendering for benchmark summaries.
//!
//! Text tables always compile. PNG plots live behind the `plot` cargo
//! feature; see [`plot_feature_required_message`] for the message the CLI
//! prints when `--plot` is passed to a binary built without that feature.

pub mod table;

#[cfg(feature = "plot")]
pub mod plot;

pub use table::format_summary_table;

#[cfg(feature = "plot")]
pub use plot::{plot_energy_cdfs, plot_sweep_heatmap, plot_ttt_cdf, PlotError};

/// Message for a `--plot` request when the binary was built without the
/// `plot` feature. Names the feature and the rebuild command so the failure
/// is actionable rather than clap's generic "unexpected argument".
#[must_use]
pub fn plot_feature_required_message() -> &'static str {
    "this binary was built without the `plot` feature; rebuild with: cargo build --features plot"
}
