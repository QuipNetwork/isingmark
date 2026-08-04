// SPDX-License-Identifier: AGPL-3.0-or-later
//! Per-job benchmark records, JSONL storage, and run summaries.
//!
//! [`JobRecord`] is one line of a benchmark run's `.jsonl` report; `job_id`
//! is hex text, matching the convention `quip-coordinator drive` itself uses
//! for the `job_id` field in its own JSONL reports (see
//! `tests/problem_golden.rs`, which parses that field as a hex string).
//! [`Summary`] is the run's closing JSON document. Unlike the Python
//! implementation this replaces, it always records [`TopologyInfo`],
//! [`HardwareInfo`], and the run `config`, so a result can be attributed to
//! the machine and problem set that produced it.

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write as _};
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

/// Outcome of a single benchmarked job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobRecord {
    /// Job id as hex text.
    pub job_id: String,
    /// Backend name that ran the job (e.g. `"cpu-sa"`, `"cuda"`).
    pub backend: String,
    /// Number of solutions the backend returned.
    pub n_solutions: usize,
    /// Best (lowest) energy among the returned solutions, in milli-units.
    pub best_energy_milli: i64,
    /// Diversity among the returned solutions, in milli-units.
    pub diversity_milli: u32,
    /// Whether the job satisfied the run's acceptance criteria.
    pub passed: bool,
    /// Whether the job was rejected outright (invalid or malformed result).
    pub rejected: bool,
    /// Reason the job was rejected, when `rejected` is `true`.
    pub reject_reason: Option<String>,
    /// Wall time from job dispatch to first device response, in
    /// microseconds.
    pub device_access_time_us: u64,
    /// Total wall time to complete the job, in milliseconds.
    pub wall_ms: u64,
    /// Reads requested for this job.
    pub reads: u32,
    /// Sweeps per read requested for this job (sweeping backends).
    pub sweeps: u32,
    /// Anneal time requested for this job, in microseconds (annealing
    /// backends).
    pub anneal_time_us: u32,
}

/// Append `rec` as one JSON line to `path`, creating the file if it does not
/// exist and appending to it if it does.
///
/// The line is written with a single `write_all` call (JSON payload and
/// trailing newline in one buffer) rather than `writeln!`'s separate writes,
/// so a process killed mid-append cannot leave a complete-but-newline-less
/// record followed by a second, unrelated write landing on the same
/// physical line. This is not formal atomicity — a crash can still land
/// inside the one `write`, and short writes are possible in principle — but
/// for a sub-4KiB append to a regular file opened in append mode, that does
/// not occur in practice, and it removes the two-syscall window entirely.
///
/// # Errors
///
/// Returns an error if `path` cannot be opened for appending, if `rec`
/// cannot be serialized, or if the write fails.
pub fn append_jsonl(path: &Path, rec: &JobRecord) -> io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let line = serde_json::to_string(rec)?;
    file.write_all(format!("{line}\n").as_bytes())
}

/// Read all records from a JSONL file at `path`, in file order.
///
/// Blank lines are skipped, and so is any line that fails to parse as a
/// [`JobRecord`] — for example a partial line left behind by a process
/// killed mid-`append_jsonl`, or that partial line merged with the next
/// record's write (see [`append_jsonl`]). A single corrupt line is data
/// loss for that one record, but it does not cost every record before or
/// after it, and the file remains readable.
///
/// # Errors
///
/// Returns an error if `path` cannot be opened, or if a line cannot be read
/// (for example, invalid UTF-8).
pub fn read_jsonl(path: &Path) -> io::Result<Vec<JobRecord>> {
    let file = File::open(path)?;
    let mut records = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(rec) = serde_json::from_str(&line) {
            records.push(rec);
        }
    }
    Ok(records)
}

/// Aggregate counts across a benchmark run's job records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Totals {
    /// Total number of jobs run.
    pub jobs: usize,
    /// Number of jobs that passed the run's acceptance criteria.
    pub passed: usize,
    /// Number of jobs that were rejected outright.
    pub rejected: usize,
}

/// Identifying shape of the topology a run benchmarked against.
///
/// `num_nodes`/`num_edges` capture the graph's size only, not its structure
/// or allowed-value sets — two topologies can share both counts and still
/// draw materially different problems. `topology_hash` disambiguates them:
/// it is the hex encoding of `topology::TopologySpec::topology_hash()`,
/// which covers sorted nodes, canonically-sorted edges, and all three
/// allowed-value sets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyInfo {
    /// Human-readable topology label (a fixture/preset name).
    pub name: String,
    /// Node count.
    pub num_nodes: usize,
    /// Edge count.
    pub num_edges: usize,
    /// Hex-encoded `topology::TopologySpec::topology_hash()`.
    pub topology_hash: String,
}

/// Sentinel for [`HardwareInfo::ram_gb`] when detection fails: real hardware
/// never reports zero or negative RAM, so this is unambiguous.
const RAM_GB_UNKNOWN: f64 = -1.0;

/// Host hardware a run executed on.
///
/// Detection never fails: any field that cannot be determined degrades to
/// `"unknown"` (or [`RAM_GB_UNKNOWN`] for `ram_gb`) rather than returning an
/// error, so a benchmark run never aborts because hardware detection did not
/// work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HardwareInfo {
    /// Operating system (`std::env::consts::OS`, e.g. `"macos"`, `"linux"`).
    pub platform: String,
    /// CPU model string, or `"unknown"` if it could not be determined.
    pub cpu: String,
    /// Total RAM in GiB, or [`RAM_GB_UNKNOWN`] if it could not be
    /// determined.
    pub ram_gb: f64,
}

impl HardwareInfo {
    /// Detect the current host's platform, CPU model, and RAM.
    #[must_use]
    pub fn detect() -> Self {
        Self {
            platform: std::env::consts::OS.to_string(),
            cpu: detect_cpu(),
            ram_gb: detect_ram_gb(),
        }
    }
}

/// Run of a benchmark, closing with the aggregate result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    /// Benchmark mode that produced this summary (e.g. `"throughput"`).
    pub mode: String,
    /// Backend name the run benchmarked.
    pub backend: String,
    /// Topology the run benchmarked against.
    pub topology: TopologyInfo,
    /// Host hardware the run executed on.
    pub hardware: HardwareInfo,
    /// Run configuration, as passed to the CLI (arbitrary JSON since shape
    /// varies by mode).
    pub config: serde_json::Value,
    /// Aggregate counts across the run's job records.
    pub totals: Totals,
}

/// Write `summary` as pretty-printed JSON to `path`, overwriting any
/// existing file.
///
/// # Errors
///
/// Returns an error if `summary` cannot be serialized or `path` cannot be
/// written.
pub fn write_summary(path: &Path, summary: &Summary) -> io::Result<()> {
    let json = serde_json::to_string_pretty(summary)?;
    std::fs::write(path, json)
}

/// Detect the CPU model, degrading to `"unknown"` on any failure.
fn detect_cpu() -> String {
    if cfg!(target_os = "macos") {
        run_command("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else if cfg!(target_os = "linux") {
        std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|text| parse_proc_cpuinfo_model(&text))
    } else {
        None
    }
    .unwrap_or_else(|| "unknown".to_string())
}

/// Detect total RAM in GiB, degrading to [`RAM_GB_UNKNOWN`] on any failure.
fn detect_ram_gb() -> f64 {
    if cfg!(target_os = "macos") {
        run_command("sysctl", &["-n", "hw.memsize"]).and_then(|out| parse_sysctl_bytes_gb(&out))
    } else if cfg!(target_os = "linux") {
        std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|text| parse_proc_meminfo_gb(&text))
    } else {
        None
    }
    .unwrap_or(RAM_GB_UNKNOWN)
}

/// Run `cmd args` and return its trimmed stdout, or `None` if the command
/// cannot be spawned, exits non-zero, or its output is not valid UTF-8.
fn run_command(cmd: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(cmd).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Convert a byte count to `f64` GiB. RAM sizes are far below `f64`'s
/// 52-bit exact-integer range, so the conversion loses no precision that
/// matters here.
#[expect(
    clippy::cast_precision_loss,
    reason = "RAM sizes are far below f64's 52-bit exact-integer range"
)]
fn bytes_to_gb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}

/// Parse `sysctl -n hw.memsize` output (a plain byte count) into GiB.
fn parse_sysctl_bytes_gb(text: &str) -> Option<f64> {
    text.trim().parse::<u64>().ok().map(bytes_to_gb)
}

/// Parse a `/proc/cpuinfo`-style `"model name"` line's value.
fn parse_proc_cpuinfo_model(text: &str) -> Option<String> {
    for line in text.lines() {
        let (key, value) = line.split_once(':')?;
        if key.trim().eq_ignore_ascii_case("model name") {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Parse a `/proc/meminfo`-style `"MemTotal: <n> kB"` line's value into
/// GiB.
fn parse_proc_meminfo_gb(text: &str) -> Option<f64> {
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("MemTotal") {
            let kb_text = value.trim().trim_end_matches("kB").trim();
            let kb: u64 = kb_text.parse().ok()?;
            return Some(bytes_to_gb(kb) * 1024.0);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        parse_proc_cpuinfo_model, parse_proc_meminfo_gb, parse_sysctl_bytes_gb, run_command,
        HardwareInfo, RAM_GB_UNKNOWN,
    };

    // Clippy sees this as an assertion on a constant since it can fold
    // `RAM_GB_UNKNOWN` at lint time; that is exactly the point, since
    // mutation testing recompiles this file with the constant's sign
    // flipped and needs a runtime check to notice.
    #[test]
    #[expect(
        clippy::assertions_on_constants,
        reason = "guards RAM_GB_UNKNOWN's sign, which mutation testing flips"
    )]
    fn ram_gb_unknown_sentinel_is_negative() {
        // Real hardware never reports negative RAM, so this must stay
        // negative to be unambiguous; a flipped sign would collide with a
        // real, if implausible, `1.0` GiB reading.
        assert!(RAM_GB_UNKNOWN < 0.0);
    }

    #[test]
    fn parse_sysctl_bytes_gb_converts_bytes_to_gib() {
        // 2^34 bytes = 16 GiB exactly.
        assert_eq!(parse_sysctl_bytes_gb("17179869184"), Some(16.0));
    }

    #[test]
    fn parse_sysctl_bytes_gb_rejects_garbage() {
        assert_eq!(parse_sysctl_bytes_gb("not-a-number"), None);
    }

    #[test]
    fn parse_proc_cpuinfo_model_extracts_model_name_line() {
        let text = "processor\t: 0\nvendor_id\t: GenuineIntel\nmodel name\t: Some CPU @ 3.00GHz\ncache size\t: 8192 KB\n";
        assert_eq!(
            parse_proc_cpuinfo_model(text),
            Some("Some CPU @ 3.00GHz".to_string())
        );
    }

    #[test]
    fn parse_proc_cpuinfo_model_returns_none_when_absent() {
        let text = "processor\t: 0\nvendor_id\t: GenuineIntel\n";
        assert_eq!(parse_proc_cpuinfo_model(text), None);
    }

    #[test]
    fn parse_proc_meminfo_gb_extracts_memtotal_line() {
        // 16777216 kB = 16 GiB exactly.
        let text = "MemTotal:       16777216 kB\nMemFree:        1234 kB\n";
        assert_eq!(parse_proc_meminfo_gb(text), Some(16.0));
    }

    #[test]
    fn parse_proc_meminfo_gb_returns_none_when_absent() {
        let text = "MemFree:        1234 kB\n";
        assert_eq!(parse_proc_meminfo_gb(text), None);
    }

    #[test]
    fn run_command_returns_none_for_missing_binary() {
        assert_eq!(run_command("definitely-not-a-real-command-xyz", &[]), None);
    }

    #[test]
    fn run_command_returns_none_for_nonzero_exit() {
        assert_eq!(run_command("false", &[]), None);
    }

    #[test]
    fn run_command_trims_stdout() {
        assert_eq!(run_command("printf", &["  hi  "]), Some("hi".to_string()));
    }

    #[test]
    fn detect_never_panics_and_reports_a_platform() {
        let info = HardwareInfo::detect();
        assert!(!info.platform.is_empty());
    }

    /// Independently re-runs the same `sysctl` calls `detect_cpu`/
    /// `detect_ram_gb` use, so this catches a hardcoded or wrong return
    /// value from either function without hardcoding this machine's own
    /// CPU/RAM values into the test.
    #[cfg(target_os = "macos")]
    #[test]
    fn detect_on_macos_matches_independent_sysctl_calls() {
        let cpu_output = run_command("sysctl", &["-n", "machdep.cpu.brand_string"])
            .expect("sysctl cpu brand lookup succeeds on macOS");
        let ram_output = run_command("sysctl", &["-n", "hw.memsize"])
            .expect("sysctl memsize lookup succeeds on macOS");
        let expected_ram_gb =
            parse_sysctl_bytes_gb(&ram_output).expect("sysctl memsize output parses");

        let info = HardwareInfo::detect();
        assert_eq!(info.platform, "macos");
        assert_eq!(info.cpu, cpu_output);
        assert!((info.ram_gb - expected_ram_gb).abs() < f64::EPSILON);
    }

    /// Independently re-parses `/proc/cpuinfo`/`/proc/meminfo`, so this
    /// catches a hardcoded or wrong return value from either function
    /// without hardcoding a specific CI host's CPU/RAM values into the
    /// test.
    #[cfg(target_os = "linux")]
    #[test]
    fn detect_on_linux_matches_independent_proc_reads() {
        let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").expect("/proc/cpuinfo is readable");
        let expected_cpu =
            parse_proc_cpuinfo_model(&cpuinfo).expect("/proc/cpuinfo has a model name line");
        let meminfo = std::fs::read_to_string("/proc/meminfo").expect("/proc/meminfo is readable");
        let expected_ram_gb =
            parse_proc_meminfo_gb(&meminfo).expect("/proc/meminfo has a MemTotal line");

        let info = HardwareInfo::detect();
        assert_eq!(info.platform, "linux");
        assert_eq!(info.cpu, expected_cpu);
        assert!((info.ram_gb - expected_ram_gb).abs() < f64::EPSILON);
    }
}
