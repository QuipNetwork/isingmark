// SPDX-License-Identifier: AGPL-3.0-or-later
//! PNG plots for sweep heatmaps, time-to-target CDFs, and energy CDFs.
//!
//! Compiles only with the `plot` cargo feature (`plotters`). The CLI still
//! accepts `--plot` without that feature so the missing-feature path can
//! print a clear rebuild command instead of clap's generic "unexpected
//! argument" error.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use plotters::prelude::*;

use crate::modes::milli_to_f64;
use crate::modes::sweep::{CellOutcome, SweepReport};
use crate::modes::ttt::TttReport;

/// Failure while rendering or writing a plot.
#[derive(Debug)]
pub enum PlotError {
    /// Underlying filesystem or bitmap I/O failure.
    Io(std::io::Error),
    /// plotters drawing backend or series construction failed.
    Drawing(String),
    /// The report held no usable samples for the requested chart.
    EmptyData(&'static str),
}

impl fmt::Display for PlotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "plot I/O error: {e}"),
            Self::Drawing(msg) => write!(f, "plot drawing error: {msg}"),
            Self::EmptyData(what) => write!(f, "cannot plot: {what}"),
        }
    }
}

impl std::error::Error for PlotError {}

impl From<std::io::Error> for PlotError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

/// Pixel size used for every chart this module writes.
const PLOT_WIDTH: u32 = 800;
const PLOT_HEIGHT: u32 = 600;

/// Heatmap of mean passed best-energy (natural units) over a sweep grid.
///
/// X axis is `num_reads`. Y axis is `num_sweeps` when any cell has a non-zero
/// sweeps value, otherwise `anneal_time_us`. Colour is the mean of each
/// cell's `passed` `best_energy_milli` values converted to natural units.
/// Cells with no passed records are left blank.
///
/// # Errors
///
/// Returns [`PlotError::EmptyData`] when `report.cells` is empty or every
/// cell has no passed records. Returns [`PlotError::Drawing`] or
/// [`PlotError::Io`] when the PNG cannot be built or written.
pub fn plot_sweep_heatmap(report: &SweepReport, out: &Path) -> Result<(), PlotError> {
    if report.cells.is_empty() {
        return Err(PlotError::EmptyData("sweep report has no cells"));
    }

    let use_sweeps = report.cells.iter().any(|c| c.num_sweeps > 0);
    let points: Vec<(u32, u32, f64)> = report
        .cells
        .iter()
        .filter_map(|cell| {
            let mean = cell_mean_energy(cell)?;
            let y = if use_sweeps {
                cell.num_sweeps
            } else {
                cell.anneal_time_us
            };
            Some((cell.num_reads, y, mean))
        })
        .collect();

    if points.is_empty() {
        return Err(PlotError::EmptyData(
            "sweep report has no passed energies to plot",
        ));
    }

    let x_values: BTreeSet<u32> = points.iter().map(|&(x, _, _)| x).collect();
    let y_values: BTreeSet<u32> = points.iter().map(|&(_, y, _)| y).collect();
    let x_labels: Vec<u32> = x_values.into_iter().collect();
    let y_labels: Vec<u32> = y_values.into_iter().collect();

    let min_e = points
        .iter()
        .map(|&(_, _, e)| e)
        .fold(f64::INFINITY, f64::min);
    let max_e = points
        .iter()
        .map(|&(_, _, e)| e)
        .fold(f64::NEG_INFINITY, f64::max);
    let range = if (max_e - min_e).abs() < f64::EPSILON {
        1.0
    } else {
        max_e - min_e
    };

    let y_axis_label = if use_sweeps {
        "num_sweeps"
    } else {
        "anneal_time_us"
    };

    let root = BitMapBackend::new(out, (PLOT_WIDTH, PLOT_HEIGHT)).into_drawing_area();
    root.fill(&WHITE)
        .map_err(|e| PlotError::Drawing(e.to_string()))?;

    let x_max = usize_to_f64(x_labels.len().max(1));
    let y_max = usize_to_f64(y_labels.len().max(1));

    let mut chart = ChartBuilder::on(&root)
        .caption("Sweep mean best energy", ("sans-serif", 28))
        .margin(20)
        .x_label_area_size(50)
        .y_label_area_size(70)
        .build_cartesian_2d(0.0..x_max, 0.0..y_max)
        .map_err(|e| PlotError::Drawing(e.to_string()))?;

    chart
        .configure_mesh()
        .disable_mesh()
        .x_desc("num_reads")
        .y_desc(y_axis_label)
        .x_label_formatter(&|v| {
            x_labels
                .get(f64_to_index(*v))
                .map_or_else(String::new, ToString::to_string)
        })
        .y_label_formatter(&|v| {
            y_labels
                .get(f64_to_index(*v))
                .map_or_else(String::new, ToString::to_string)
        })
        .draw()
        .map_err(|e| PlotError::Drawing(e.to_string()))?;

    for &(x_val, y_val, energy) in &points {
        let Some(xi) = x_labels.iter().position(|&v| v == x_val) else {
            continue;
        };
        let Some(yi) = y_labels.iter().position(|&v| v == y_val) else {
            continue;
        };
        let t = ((energy - min_e) / range).clamp(0.0, 1.0);
        let color = HSLColor(0.7 * (1.0 - t), 0.7, 0.45);
        let x0 = usize_to_f64(xi);
        let y0 = usize_to_f64(yi);
        chart
            .draw_series(std::iter::once(Rectangle::new(
                [(x0, y0), (x0 + 1.0, y0 + 1.0)],
                color.filled(),
            )))
            .map_err(|e| PlotError::Drawing(e.to_string()))?;
    }

    root.present()
        .map_err(|e| PlotError::Drawing(e.to_string()))?;
    Ok(())
}

/// Empirical CDF of time-to-target seconds for every target in `report`.
///
/// Each non-empty target gets one step series. Empty targets (never reached)
/// are omitted from the chart but do not alone cause an error when at least
/// one target has times.
///
/// # Errors
///
/// Returns [`PlotError::EmptyData`] when every target has an empty times
/// vector. Returns drawing or I/O errors when the PNG cannot be written.
pub fn plot_ttt_cdf(report: &TttReport, out: &Path) -> Result<(), PlotError> {
    let series: Vec<(i64, Vec<f64>)> = report
        .per_target
        .iter()
        .filter(|(_, times)| !times.is_empty())
        .map(|(&target, times)| {
            let mut sorted = times.clone();
            sorted.sort_by(f64::total_cmp);
            (target, sorted)
        })
        .collect();

    if series.is_empty() {
        return Err(PlotError::EmptyData(
            "ttt report has no reached-target times to plot",
        ));
    }

    let max_t = series
        .iter()
        .flat_map(|(_, times)| times.iter().copied())
        .fold(0.0_f64, f64::max)
        .max(1e-9);

    let root = BitMapBackend::new(out, (PLOT_WIDTH, PLOT_HEIGHT)).into_drawing_area();
    root.fill(&WHITE)
        .map_err(|e| PlotError::Drawing(e.to_string()))?;

    let mut chart = ChartBuilder::on(&root)
        .caption("Time-to-target CDF", ("sans-serif", 28))
        .margin(20)
        .x_label_area_size(50)
        .y_label_area_size(60)
        .build_cartesian_2d(0.0..max_t * 1.05, 0.0..1.05)
        .map_err(|e| PlotError::Drawing(e.to_string()))?;

    chart
        .configure_mesh()
        .x_desc("time (s)")
        .y_desc("CDF")
        .draw()
        .map_err(|e| PlotError::Drawing(e.to_string()))?;

    let palette = [
        &RED,
        &BLUE,
        &GREEN,
        &MAGENTA,
        &CYAN,
        &BLACK,
        &RGBColor(255, 128, 0),
        &RGBColor(128, 0, 255),
    ];

    for (idx, (target, times)) in series.iter().enumerate() {
        let n = usize_to_f64(times.len());
        let points: Vec<(f64, f64)> = times
            .iter()
            .enumerate()
            .map(|(i, &t)| (t, usize_to_f64(i + 1) / n))
            .collect();
        let color = palette[idx % palette.len()];
        let label = format!("target {target} mE");
        chart
            .draw_series(LineSeries::new(points, color.stroke_width(2)))
            .map_err(|e| PlotError::Drawing(e.to_string()))?
            .label(label)
            .legend(move |(x, y)| {
                PathElement::new(vec![(x, y), (x + 20, y)], color.stroke_width(2))
            });
    }

    chart
        .configure_series_labels()
        .border_style(BLACK)
        .background_style(WHITE.mix(0.8))
        .draw()
        .map_err(|e| PlotError::Drawing(e.to_string()))?;

    root.present()
        .map_err(|e| PlotError::Drawing(e.to_string()))?;
    Ok(())
}

/// Overlayed empirical CDFs of two energy samples (natural units).
///
/// # Errors
///
/// Returns [`PlotError::EmptyData`] when either slice is empty. Returns
/// drawing or I/O errors when the PNG cannot be written.
pub fn plot_energy_cdfs(a: &[f64], b: &[f64], out: &Path) -> Result<(), PlotError> {
    if a.is_empty() {
        return Err(PlotError::EmptyData("file A has no energies"));
    }
    if b.is_empty() {
        return Err(PlotError::EmptyData("file B has no energies"));
    }

    let mut sa: Vec<f64> = a.to_vec();
    let mut sb: Vec<f64> = b.to_vec();
    sa.sort_by(f64::total_cmp);
    sb.sort_by(f64::total_cmp);

    let min_e = sa[0].min(sb[0]);
    let max_e = sa[sa.len() - 1].max(sb[sb.len() - 1]);
    let span = if (max_e - min_e).abs() < f64::EPSILON {
        1.0
    } else {
        max_e - min_e
    };
    let x0 = min_e - 0.05 * span;
    let x1 = max_e + 0.05 * span;

    let root = BitMapBackend::new(out, (PLOT_WIDTH, PLOT_HEIGHT)).into_drawing_area();
    root.fill(&WHITE)
        .map_err(|e| PlotError::Drawing(e.to_string()))?;

    let mut chart = ChartBuilder::on(&root)
        .caption("Energy CDFs", ("sans-serif", 28))
        .margin(20)
        .x_label_area_size(50)
        .y_label_area_size(60)
        .build_cartesian_2d(x0..x1, 0.0..1.05)
        .map_err(|e| PlotError::Drawing(e.to_string()))?;

    chart
        .configure_mesh()
        .x_desc("energy (natural units)")
        .y_desc("CDF")
        .draw()
        .map_err(|e| PlotError::Drawing(e.to_string()))?;

    let na = usize_to_f64(sa.len());
    let pts_a: Vec<(f64, f64)> = sa
        .iter()
        .enumerate()
        .map(|(i, &e)| (e, usize_to_f64(i + 1) / na))
        .collect();
    chart
        .draw_series(LineSeries::new(pts_a, RED.stroke_width(2)))
        .map_err(|e| PlotError::Drawing(e.to_string()))?
        .label("file A")
        .legend(|(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], RED.stroke_width(2)));

    let nb = usize_to_f64(sb.len());
    let pts_b: Vec<(f64, f64)> = sb
        .iter()
        .enumerate()
        .map(|(i, &e)| (e, usize_to_f64(i + 1) / nb))
        .collect();
    chart
        .draw_series(LineSeries::new(pts_b, BLUE.stroke_width(2)))
        .map_err(|e| PlotError::Drawing(e.to_string()))?
        .label("file B")
        .legend(|(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], BLUE.stroke_width(2)));

    chart
        .configure_series_labels()
        .border_style(BLACK)
        .background_style(WHITE.mix(0.8))
        .draw()
        .map_err(|e| PlotError::Drawing(e.to_string()))?;

    root.present()
        .map_err(|e| PlotError::Drawing(e.to_string()))?;
    Ok(())
}

/// Mean of a cell's passed `best_energy_milli` values in natural units, or
/// `None` when the cell has no passed records with a present energy.
fn cell_mean_energy(cell: &CellOutcome) -> Option<f64> {
    let mut sum = 0.0;
    let mut n = 0_usize;
    for record in &cell.report.records {
        if record.passed {
            if let Some(energy) = record.best_energy_milli {
                sum += milli_to_f64(energy);
                n += 1;
            }
        }
    }
    if n == 0 {
        None
    } else {
        Some(sum / usize_to_f64(n))
    }
}

/// Convert a count or index to `f64` for chart axes. Plot sample sizes and
/// grid dimensions are far below the 52-bit exact-integer range of `f64`.
#[expect(
    clippy::cast_precision_loss,
    reason = "plot sample counts and grid sizes are far below f64 exact-integer range"
)]
fn usize_to_f64(n: usize) -> f64 {
    n as f64
}

/// Convert a chart-axis coordinate (non-negative, small integer) to a slice
/// index for label lookup.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "axis labels are small non-negative integer grid indices"
)]
fn f64_to_index(v: f64) -> usize {
    v as usize
}
