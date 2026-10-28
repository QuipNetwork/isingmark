// SPDX-License-Identifier: AGPL-3.0-or-later
//! Versioned campaign traces and separate device-time bounds.

use std::{collections::BTreeMap, fs, io::Write, path::PathBuf, time::Duration};

use isingmark::backend::{
    run_session_with_capture, BackendSpec, BatchCapture, RunParams, RunReport,
};
use isingmark::campaign::{
    contracts::TimingStatus,
    trace::{read_trace, DeviceTime, EventKind, TraceEvent, TraceRead, TraceWriter, TRACE_VERSION},
    CampaignError,
};
use isingmark::params::BackendKind;
use isingmark::topology::{load_fixture, TopologySpec};
use quip_proto::v1::{ising_problem, CoefficientEncoding, IsingProblem, Job, JobKind};
use quip_protocol::wire::encode_i32_le;
use serde_json::json;

fn event(seq: u64) -> TraceEvent {
    TraceEvent {
        version: TRACE_VERSION,
        run_id: "run-1".to_owned(),
        trial_id: "trial-2".to_owned(),
        session_id: "session-3".to_owned(),
        seq,
        observer: "controller".to_owned(),
        kind: EventKind::BatchReceipt,
        wall_ns: 80_000_000_000,
        instance_id: Some("instance-4".to_owned()),
        job_id: Some("job-5".to_owned()),
        batch_id: Some("batch-6".to_owned()),
        payload: json!({"reads": [{"spins": [1, -1], "energy_milli": -1000}]}),
        device: Some(DeviceTime::from_truncated_us(29_000_000)),
        anneal_ns: Some(1_234_000),
        estimate_provenance: Some("raw timing record".to_owned()),
        resources: BTreeMap::from([("reads".to_owned(), 1), ("cpu_threads".to_owned(), 2)]),
    }
}

fn read_bytes(bytes: &[u8]) -> Result<TraceRead, CampaignError> {
    let directory = tempfile::tempdir().expect("create trace directory");
    let path = directory.path().join("trace.jsonl");
    fs::write(&path, bytes).expect("write trace bytes");
    read_trace(&path)
}

#[test]
fn device_time_is_not_wall_time() {
    let directory = tempfile::tempdir().expect("create trace directory");
    let path = directory.path().join("trace.jsonl");
    let mut writer = TraceWriter::create(&path).expect("create trace writer");
    let expected = event(1);
    writer.append(&expected).expect("write complete event");

    // Read while the writer is alive to check that append flushes each event.
    let trace = read_trace(&path).expect("read flushed event");
    assert!(!trace.truncated_tail);
    assert_eq!(trace.events, [expected]);
    let saved = &trace.events[0];
    assert_eq!(saved.wall_ns, 80_000_000_000);
    assert_eq!(
        saved.device,
        Some(DeviceTime {
            lower_ns: 29_000_000_000,
            upper_ns: Some(29_000_000_999),
            status: TimingStatus::Complete,
        })
    );
    let contents = fs::read_to_string(&path).expect("read raw trace");
    assert!(contents.ends_with('\n'));
    assert_eq!(contents.lines().count(), 1);
}

#[test]
fn missing_device_time_stays_missing() {
    let total = DeviceTime::missing()
        .add(DeviceTime::from_exact_ns(5))
        .expect("sum known and missing costs");
    assert_eq!(total.lower_ns, 5);
    assert_eq!(total.upper_ns, None);
    assert_eq!(total.status, TimingStatus::Missing);
}

#[test]
fn truncated_microseconds_keep_a_rounding_bound() {
    let total = DeviceTime::from_truncated_us(10)
        .add(DeviceTime::from_truncated_us(10))
        .expect("sum first two submissions")
        .add(DeviceTime::from_truncated_us(10))
        .expect("sum third submission");
    assert_eq!(total.lower_ns, 30_000);
    assert_eq!(total.upper_ns, Some(32_997));
    assert_eq!(total.status, TimingStatus::Complete);
}

#[test]
fn malformed_trace_is_rejected() {
    let directory = tempfile::tempdir().expect("create trace directory");
    let path = directory.path().join("trace.jsonl");
    let mut writer = TraceWriter::create(&path).expect("create trace writer");
    writer.append(&event(1)).expect("write first event");
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open trace for corruption");
    writeln!(file, "not JSON").expect("write malformed middle line");
    serde_json::to_writer(&mut file, &event(3)).expect("write third event");
    writeln!(file).expect("terminate third line");

    let error = read_trace(&path).expect_err("reject damaged middle record");
    assert!(matches!(error, CampaignError::InvalidRun(_)));
}

#[test]
fn truncated_final_record_is_reported() {
    let directory = tempfile::tempdir().expect("create trace directory");
    let path = directory.path().join("trace.jsonl");
    let mut writer = TraceWriter::create(&path).expect("create trace writer");
    writer.append(&event(1)).expect("write first event");
    writer.append(&event(2)).expect("write second event");
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open trace for partial write");
    file.write_all(b"{\"version\":1,\"run_id\":")
        .expect("write partial final event");

    let trace = read_trace(&path).expect("keep events before truncated tail");
    assert!(trace.truncated_tail);
    assert_eq!(trace.events, [event(1), event(2)]);
}

#[test]
fn sequence_must_increase() {
    let directory = tempfile::tempdir().expect("create trace directory");
    let path = directory.path().join("trace.jsonl");
    let mut writer = TraceWriter::create(&path).expect("create trace writer");
    writer.append(&event(5)).expect("write initial sequence");
    let error = writer
        .append(&event(5))
        .expect_err("reject duplicate sequence");
    assert!(matches!(error, CampaignError::InvalidRun(_)));
    writer
        .append(&event(6))
        .expect("continue after rejected event");
    assert_eq!(
        read_trace(&path).expect("read valid sequence").events,
        [event(5), event(6)]
    );
}

#[test]
fn complete_final_record_without_newline_is_kept() {
    let contents = format!(
        "{}\n{}",
        serde_json::to_string(&event(1)).expect("serialize first event"),
        serde_json::to_string(&event(2)).expect("serialize final event")
    );
    let trace = read_bytes(contents.as_bytes()).expect("read complete final event");
    assert!(!trace.truncated_tail);
    assert_eq!(trace.events, [event(1), event(2)]);
}

#[test]
fn unsupported_trace_versions_are_rejected_before_event_fields() {
    for version in [0, 2, u32::MAX] {
        for suffix in ["", "\n"] {
            let contents = format!("{{\"version\":{version},\"kind\":\"future_event\"}}{suffix}");
            let error = read_bytes(contents.as_bytes()).expect_err("reject unsupported schema");
            assert!(matches!(error, CampaignError::UnsupportedVersion(found) if found == version));
        }
    }
}

#[test]
fn malformed_newline_terminated_final_record_is_rejected() {
    let first = serde_json::to_string(&event(1)).expect("serialize first event");
    for bad in ["not JSON", "{\"version\":1,", ""] {
        let contents = format!("{first}\n{bad}\n");
        assert!(matches!(
            read_bytes(contents.as_bytes()),
            Err(CampaignError::InvalidRun(_))
        ));
    }
}

#[test]
fn complete_json_with_invalid_event_fields_is_rejected() {
    for suffix in ["", "\n"] {
        let contents = format!("{{\"version\":1}}{suffix}");
        assert!(matches!(
            read_bytes(contents.as_bytes()),
            Err(CampaignError::InvalidRun(_))
        ));
    }
}

#[test]
fn reader_rejects_non_increasing_sequences() {
    for seq in [4, 5] {
        for suffix in ["", "\n"] {
            let contents = format!(
                "{}\n{}{suffix}",
                serde_json::to_string(&event(5)).expect("serialize first event"),
                serde_json::to_string(&event(seq)).expect("serialize out-of-order event")
            );
            assert!(matches!(
                read_bytes(contents.as_bytes()),
                Err(CampaignError::InvalidRun(_))
            ));
        }
    }
}

#[test]
fn writer_rejects_decreasing_sequences_and_accepts_u64_endpoints() {
    let directory = tempfile::tempdir().expect("create trace directory");
    let path = directory.path().join("trace.jsonl");
    let mut writer = TraceWriter::create(&path).expect("create trace writer");
    writer.append(&event(0)).expect("accept sequence zero");
    writer.append(&event(5)).expect("allow sequence gaps");
    assert!(matches!(
        writer.append(&event(4)),
        Err(CampaignError::InvalidRun(_))
    ));
    writer
        .append(&event(u64::MAX))
        .expect("accept largest sequence without incrementing it");
    assert_eq!(
        read_trace(&path).expect("read full sequence range").events,
        [event(0), event(5), event(u64::MAX)]
    );
}

#[test]
fn device_addition_checks_lower_and_upper_overflow() {
    for left in [
        DeviceTime::from_exact_ns(u64::MAX),
        DeviceTime {
            lower_ns: 0,
            upper_ns: Some(u64::MAX),
            status: TimingStatus::ConservativeEstimate,
        },
    ] {
        assert!(matches!(
            left.add(DeviceTime::from_exact_ns(1)),
            Err(CampaignError::Overflow)
        ));
    }
    let largest = DeviceTime::from_exact_ns(u64::MAX)
        .add(DeviceTime::from_exact_ns(0))
        .expect("accept exact boundary sum");
    assert_eq!(largest.lower_ns, u64::MAX);
    assert_eq!(largest.upper_ns, Some(u64::MAX));
}

#[test]
fn device_addition_takes_the_worst_status_and_preserves_unknown_bounds() {
    use TimingStatus::{Complete, ConservativeEstimate, Missing};

    for (left, right, expected) in [
        (Complete, Complete, Complete),
        (Complete, ConservativeEstimate, ConservativeEstimate),
        (Complete, Missing, Missing),
        (ConservativeEstimate, Complete, ConservativeEstimate),
        (
            ConservativeEstimate,
            ConservativeEstimate,
            ConservativeEstimate,
        ),
        (ConservativeEstimate, Missing, Missing),
        (Missing, Complete, Missing),
        (Missing, ConservativeEstimate, Missing),
        (Missing, Missing, Missing),
    ] {
        let total = DeviceTime {
            lower_ns: 2,
            upper_ns: Some(3),
            status: left,
        }
        .add(DeviceTime {
            lower_ns: 5,
            upper_ns: Some(7),
            status: right,
        })
        .expect("sum each timing status combination");
        assert_eq!(total.lower_ns, 7);
        assert_eq!(total.upper_ns, Some(10));
        assert_eq!(total.status, expected);
    }
    let unknown = DeviceTime {
        lower_ns: 3,
        upper_ns: None,
        status: ConservativeEstimate,
    };
    for (left, right) in [
        (unknown, DeviceTime::from_exact_ns(5)),
        (DeviceTime::from_exact_ns(5), unknown),
    ] {
        let total = left.add(right).expect("keep unknown upper bound");
        assert_eq!(total.lower_ns, 8);
        assert_eq!(total.upper_ns, None);
        assert_eq!(total.status, ConservativeEstimate);
    }
}

#[test]
fn unrepresentable_microsecond_bounds_stay_missing() {
    for us in [18_446_744_073_709_551, 18_446_744_073_709_552, u64::MAX] {
        assert_eq!(DeviceTime::from_truncated_us(us), DeviceTime::missing());
    }
    assert_eq!(
        DeviceTime::from_truncated_us(18_446_744_073_709_550),
        DeviceTime {
            lower_ns: 18_446_744_073_709_550_000,
            upper_ns: Some(18_446_744_073_709_550_999),
            status: TimingStatus::Complete,
        }
    );
}

#[test]
fn every_event_kind_round_trips_without_optional_fields() {
    let directory = tempfile::tempdir().expect("create trace directory");
    let path = directory.path().join("trace.jsonl");
    let mut writer = TraceWriter::create(&path).expect("create trace writer");
    let mut expected = Vec::new();
    for (seq, kind) in (0..).zip([
        EventKind::ChallengeRelease,
        EventKind::JobDispatch,
        EventKind::BatchReceipt,
        EventKind::WitnessValidation,
        EventKind::ProofAcceptance,
        EventKind::Rejection,
        EventKind::TargetChange,
        EventKind::Cancellation,
        EventKind::TrialTermination,
    ]) {
        let mut event = event(seq);
        event.kind = kind;
        event.instance_id = None;
        event.job_id = None;
        event.batch_id = None;
        event.device = None;
        event.anneal_ns = None;
        event.estimate_provenance = None;
        writer.append(&event).expect("append each event kind");
        expected.push(event);
    }
    let trace = read_trace(&path).expect("read every event kind");
    assert!(!trace.truncated_tail);
    assert_eq!(trace.events, expected);
    for line in fs::read_to_string(&path)
        .expect("read serialized events")
        .lines()
    {
        let value: serde_json::Value = serde_json::from_str(line).expect("read event envelope");
        assert_eq!(value["version"], json!(1));
        assert!(value["device"].is_null());
    }
}

#[test]
fn empty_trace_has_no_truncated_tail() {
    let trace = read_bytes(b"").expect("read empty trace");
    assert!(trace.events.is_empty());
    assert!(!trace.truncated_tail);
}

fn capture_job(spec: &TopologySpec, id: u8) -> Job {
    Job {
        job_id: vec![id],
        kind: JobKind::IsingSample as i32,
        ising: Some(IsingProblem {
            graph: Some(ising_problem::Graph::TopologyHash(spec.topology_hash())),
            encoding: CoefficientEncoding::I32 as i32,
            scale: 1000,
            h: encode_i32_le(&vec![1000; spec.nodes.len()]),
            j: encode_i32_le(&vec![1000; spec.edges.len()]),
            num_sweeps: 17,
            ..Default::default()
        }),
        ..Default::default()
    }
}

async fn capture_jobs(
    spec: &TopologySpec,
    jobs: Vec<Job>,
    args: &[&str],
) -> (RunReport, Vec<BatchCapture>) {
    let directory = tempfile::tempdir().expect("create session directory");
    let backend = BackendSpec {
        name: "fake".to_owned(),
        bin: PathBuf::from(env!("CARGO_BIN_EXE_isingmark-fake-miner")),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    };
    run_session_with_capture(RunParams {
        backend: &backend,
        spec,
        jobs,
        sock_path: &directory.path().join("coordinator.sock"),
        target: None,
    })
    .await
    .expect("capture fake miner session")
}

#[tokio::test]
async fn batch_round_trip_preserves_all_reads() {
    let spec = load_fixture("smoke").expect("load smoke topology");
    let (report, captures) = capture_jobs(
        &spec,
        vec![capture_job(&spec, 1)],
        &[
            "--num-solutions",
            "5",
            "--qpu-access-us",
            "61319",
            "--meta-extra",
            "qpu_access_time=61319.84",
            "--meta-extra",
            "qpu_access_overhead_time=0",
        ],
    )
    .await;
    assert!(report.handshake_ok);
    assert_eq!(report.miner_exit_code, 0);
    assert_eq!(report.records.len(), 1);
    assert!(report.records[0].passed);
    assert_eq!(report.records[0].n_solutions, 5);
    assert_eq!(report.records[0].best_energy_milli, Some(4000));
    assert_eq!(captures.len(), 1);
    let capture = &captures[0];
    assert_eq!(capture.job_id, [1]);
    assert_eq!(capture.solutions.len(), 5);
    // Solution i flips the first i spins. These are all five wire reads,
    // including the four that the legacy record's best-of-N reduction drops.
    let expected_spins = [
        [1, 1, 1, 1, 1, 1, 1, 1],
        [-1, 1, 1, 1, 1, 1, 1, 1],
        [-1, -1, 1, 1, 1, 1, 1, 1],
        [-1, -1, -1, 1, 1, 1, 1, 1],
        [-1, -1, -1, -1, 1, 1, 1, 1],
    ];
    // Eight +1000 fields and eight +1000 ring couplings: flipping a
    // proper prefix leaves two negative couplings and removes two fields per flip.
    for ((solution, spins), energy) in capture
        .solutions
        .iter()
        .zip(expected_spins)
        .zip([16000, 10000, 8000, 6000, 4000])
    {
        assert_eq!(solution.spins, spins);
        assert_eq!(solution.reported_energy_milli, energy);
        assert_eq!(solution.recomputed_energy_milli, energy);
    }
    let meta = capture.meta.as_ref().expect("fake miner reports metadata");
    assert_eq!(meta.reads, 5);
    assert_eq!(meta.sweeps, 17);
    assert_eq!(meta.device_access_time_us, 0);
    assert_eq!(meta.qpu_access_us, 61319);
    assert_eq!(
        meta.extra,
        BTreeMap::from([
            ("qpu_access_time".to_owned(), "61319.84".to_owned()),
            ("qpu_access_overhead_time".to_owned(), "0".to_owned()),
        ])
    );

    let mut receipt = event(1);
    receipt.job_id = Some(report.records[0].job_id.clone());
    receipt.wall_ns = u64::try_from(capture.received_after_dispatch.as_nanos())
        .expect("test receipt fits nanoseconds");
    receipt.device = Some(DeviceTime::from_truncated_us(meta.qpu_access_us));
    receipt.anneal_ns = None;
    receipt.estimate_provenance = None;
    receipt.resources = BTreeMap::from([("reads".to_owned(), u64::from(meta.reads))]);
    let reads: Vec<_> = capture
        .solutions
        .iter()
        .map(|solution| {
            json!({
                "spins": solution.spins,
                "reported_energy_milli": solution.reported_energy_milli,
                "recomputed_energy_milli": solution.recomputed_energy_milli,
            })
        })
        .collect();
    receipt.payload = json!({
        "reads": reads,
        "meta": {
            "reads": meta.reads,
            "sweeps": meta.sweeps,
            "device_access_time_us": meta.device_access_time_us,
            "qpu_access_us": meta.qpu_access_us,
            "extra": meta.extra,
        },
    });
    let directory = tempfile::tempdir().expect("create trace directory");
    let path = directory.path().join("captured.jsonl");
    let mut writer = TraceWriter::create(&path).expect("create trace writer");
    writer
        .append(&receipt)
        .expect("write captured batch receipt");
    let trace = read_trace(&path).expect("read captured batch receipt");
    assert!(!trace.truncated_tail);
    assert_eq!(trace.events, [receipt]);
}

#[tokio::test]
async fn batch_capture_recomputes_each_duplicate_dispatch_and_keeps_reported_energy() {
    let mut spec = load_fixture("smoke").expect("load smoke topology");
    spec.nodes = vec![10, 20, 30, 40, 50, 60, 70, 80];
    spec.edges = vec![
        (10, 20),
        (20, 30),
        (30, 40),
        (40, 50),
        (50, 60),
        (60, 70),
        (70, 80),
        (80, 10),
    ];
    let first = capture_job(&spec, 1);
    let mut second = first.clone();
    second.ising.as_mut().expect("job has coefficients").h = encode_i32_le(&[-1000; 8]);
    let (report, captures) = capture_jobs(
        &spec,
        vec![first, second],
        &["--credits", "2", "--energy-lie-milli", "500"],
    )
    .await;
    assert_eq!(report.miner_exit_code, 0);
    assert_eq!(report.records.len(), 2);
    assert!(report.records.iter().all(|record| !record.passed));
    assert_eq!(captures.len(), 2);
    for (capture, energy) in captures.iter().zip([16000, 0]) {
        assert_eq!(capture.job_id, [1]);
        assert_eq!(capture.solutions.len(), 1);
        assert_eq!(capture.solutions[0].spins, [1; 8]);
        assert_eq!(capture.solutions[0].recomputed_energy_milli, energy);
        assert_eq!(capture.solutions[0].reported_energy_milli, energy + 500);
        let meta = capture.meta.as_ref().expect("default metadata present");
        assert_eq!(meta.qpu_access_us, 0);
        assert!(meta.extra.is_empty());
    }
}

#[tokio::test]
async fn batch_capture_measures_each_dispatch_to_delayed_receipt() {
    let spec = load_fixture("smoke").expect("load smoke topology");
    let (report, captures) = capture_jobs(
        &spec,
        vec![capture_job(&spec, 1), capture_job(&spec, 2)],
        &["--result-delay-ms", "150"],
    )
    .await;
    assert_eq!(report.miner_exit_code, 0);
    assert_eq!(captures.len(), 2);
    for capture in &captures {
        assert!(capture.received_after_dispatch >= Duration::from_millis(150));
    }
    // Default credit 1 makes dispatch intervals disjoint. Measuring from
    // session start or at shutdown would count the first delay twice.
    let total: Duration = captures.iter().map(|c| c.received_after_dispatch).sum();
    assert!(total <= Duration::from_millis(report.run_wall_ms + 1));
}

#[tokio::test]
async fn batch_capture_omits_rejected_abandoned_and_orphan_results() {
    let spec = load_fixture("smoke").expect("load smoke topology");
    let jobs = (1..=4).map(|id| capture_job(&spec, id)).collect();
    let (report, captures) = capture_jobs(
        &spec,
        jobs,
        &[
            "--credits",
            "4",
            "--reject-job",
            "2",
            "--fatal-after",
            "3",
            "--stray-result-after",
            "1",
        ],
    )
    .await;
    assert!(report.handshake_ok);
    assert_eq!(report.miner_exit_code, 70);
    assert_eq!(report.orphan_results, 1);
    assert_eq!(report.records.len(), 4);
    assert!(report.records[0].passed);
    let rejected = report
        .records
        .iter()
        .find(|r| r.job_id == "02")
        .expect("rejected record");
    assert!(rejected.rejected);
    assert_eq!(rejected.reject_reason.as_deref(), Some("MALFORMED"));
    for record in report
        .records
        .iter()
        .filter(|r| r.job_id == "03" || r.job_id == "04")
    {
        assert!(!record.passed);
        assert_eq!(record.n_solutions, 0);
        assert!(record
            .reject_reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with("fatal:")));
    }
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].job_id, [1]);
}
