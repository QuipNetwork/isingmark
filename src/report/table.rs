// SPDX-License-Identifier: AGPL-3.0-or-later
//! Plain-text table rendering of a [`crate::record::Summary`].

use std::fmt::Write as _;

use crate::record::Summary;

/// Render `summary` as a small, human-readable plain-text table. Returns a
/// `String` rather than printing directly: this crate's `print-stdout`
/// lint denies scattered `println!` calls, so every caller routes the
/// result through the single documented CLI output boundary in
/// `src/main.rs` instead.
#[must_use]
pub fn format_summary_table(summary: &Summary) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "mode:      {}", summary.mode);
    let _ = writeln!(out, "backend:   {}", summary.backend);
    let _ = writeln!(
        out,
        "topology:  {} (nodes={}, edges={}, hash={})",
        summary.topology.name,
        summary.topology.num_nodes,
        summary.topology.num_edges,
        summary.topology.topology_hash,
    );
    let _ = writeln!(
        out,
        "hardware:  {} / {} / {}",
        summary.hardware.platform,
        summary.hardware.cpu,
        format_ram(summary.hardware.ram_gb),
    );
    let _ = writeln!(
        out,
        "jobs:      {} total, {} passed, {} rejected, {} skipped lines",
        summary.totals.jobs,
        summary.totals.passed,
        summary.totals.rejected,
        summary.totals.skipped_lines,
    );
    out
}

/// Render `ram_gb` as `"N.N GiB RAM"`, or `"unknown RAM"` for
/// `HardwareInfo::detect`'s documented negative sentinel.
fn format_ram(ram_gb: f64) -> String {
    if ram_gb < 0.0 {
        "unknown RAM".to_string()
    } else {
        format!("{ram_gb:.1} GiB RAM")
    }
}

#[cfg(test)]
mod tests {
    use super::format_summary_table;
    use crate::record::{HardwareInfo, Summary, TopologyInfo, Totals};

    /// Every field is a distinct, nonzero value so a mutation that swaps
    /// two same-typed fields (e.g. `passed`/`rejected`, `num_nodes`/
    /// `num_edges`) is detectable in the rendered text.
    fn summary() -> Summary {
        Summary {
            mode: "throughput".to_string(),
            backend: "cpu-sa".to_string(),
            topology: TopologyInfo {
                name: "smoke".to_string(),
                num_nodes: 8,
                num_edges: 5,
                topology_hash: "deadbeef".to_string(),
            },
            hardware: HardwareInfo {
                platform: "macos".to_string(),
                cpu: "Apple M4 Max".to_string(),
                ram_gb: 64.0,
            },
            config: serde_json::json!({}),
            totals: Totals {
                jobs: 17,
                passed: 13,
                rejected: 4,
                skipped_lines: 2,
            },
        }
    }

    #[test]
    fn table_contains_every_summary_field_with_its_exact_value() {
        let table = format_summary_table(&summary());
        assert!(table.contains("throughput"));
        assert!(table.contains("cpu-sa"));
        assert!(table.contains("smoke"));
        assert!(table.contains("nodes=8"));
        assert!(table.contains("edges=5"));
        assert!(table.contains("deadbeef"));
        assert!(table.contains("macos"));
        assert!(table.contains("Apple M4 Max"));
        assert!(table.contains("64.0 GiB RAM"));
        assert!(table.contains("17 total"));
        assert!(table.contains("13 passed"));
        assert!(table.contains("4 rejected"));
        assert!(table.contains("2 skipped lines"));
    }

    #[test]
    fn format_ram_reports_unknown_for_the_negative_sentinel() {
        let mut summary = summary();
        summary.hardware.ram_gb = -1.0;
        let table = format_summary_table(&summary);
        assert!(table.contains("unknown RAM"));
        assert!(!table.contains("-1.0 GiB RAM"));
    }
}
