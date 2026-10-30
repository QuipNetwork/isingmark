// SPDX-License-Identifier: AGPL-3.0-or-later
//! Challenge-relative trials and serial campaigns over the miner session transport.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant, SystemTime},
};

use quip_proto::v1::{ising_problem, CoefficientEncoding, IsingProblem, Job, JobKind, SetTarget};
use quip_protocol::wire::encode_i32_le;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{
    backend::{
        builtin_registry,
        control::{self, SessionCommand, SessionControl, SessionEvent, Submission},
        BackendSpec, BatchCapture, RunParams, RunReport, SessionError,
    },
    params::BackendKind,
    topology::TopologySpec,
};

use super::{
    contracts::{EncodingOutcome, FamilyEvaluator, IsingEncoding, Witness},
    families::fixed::FixedIsing,
    instance::{load_instance, Instance},
    protocol::{
        load_protocol_profile, select_proof, target_from_objective, ProofDecision, ProtocolProfile,
        RuleSet,
    },
    timing::{BudgetOutcome, DeviceLedger, TrialBudget},
    trace::{DeviceTime, EventKind, TraceEvent, TraceWriter, TRACE_VERSION},
    CampaignError,
};

mod serve;
pub use serve::serve_campaign;
#[cfg(test)]
mod tests;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);
const PREPARATION_TIMEOUT: Duration = Duration::from_mins(2);

#[derive(Debug, Clone)]
pub struct CampaignConfig {
    pub manifest: PathBuf,
    pub backend: String,
    pub output: PathBuf,
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyManifest {
    pub policy_id: String,
    pub backend: String,
    pub num_reads: u32,
    pub num_sweeps: u32,
    pub anneal_time_us: u32,
    pub host_compute_limit_s: f64,
    pub evaluation_limit: u64,
    pub preparation: Preparation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preparation {
    Cold,
    Warm,
    Amortized { horizon: u32 },
}

/// Why the trial loop stopped, independent of proof acceptance and either clock's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalReason {
    Accepted,
    WallTimeout,
    DeviceBudgetExhausted,
    HostComputeLimit,
    EvaluationLimit,
    MinerCrashed,
    MinerRejected,
    Cancelled,
    InvalidRun,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trial {
    pub trial_id: String,
    pub instance_id: String,
    pub policy_id: String,
    pub started: SystemTime,
    pub wall_limit: Option<Duration>,
    pub device_budget_ns: Option<u64>,
    pub resources: BTreeMap<String, u64>,
    pub events: PathBuf,
    /// Challenge-relative proof completion, including proofs scored after a stop.
    pub accepted_wall: Option<Duration>,
    pub device_total: DeviceTime,
    pub terminal: TerminalReason,
    pub preparation: Preparation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceEstimate {
    pub trials: u64,
    /// An absent limit serializes as null (an unbounded estimate).
    pub max_wall_s: Option<f64>,
    pub max_device_s: Option<f64>,
    pub backends: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct CampaignReport {
    pub resources: ResourceEstimate,
    pub trials: Vec<Trial>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    protocol_profile: PathBuf,
    instances: Vec<PathBuf>,
    policies: Vec<PolicyManifest>,
    trials_per_pair: u32,
    budget: ManifestBudget,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestBudget {
    wall_limit_s: Option<f64>,
    device_budget_s: Option<f64>,
}

struct LoadedCampaign {
    instances: Vec<Instance>,
    policies: Vec<PolicyManifest>,
    trials_per_pair: u32,
    budget: TrialBudget,
    profile: ProtocolProfile,
    estimate: ResourceEstimate,
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Accept owned errors directly as a map_err callback without allocating an intermediate string."
)]
fn invalid(reason: impl ToString) -> CampaignError {
    CampaignError::InvalidRun(reason.to_string())
}

fn duration(seconds: f64) -> Result<Duration, CampaignError> {
    Duration::try_from_secs_f64(seconds).map_err(invalid)
}

fn nanos(value: Duration) -> Result<u64, CampaignError> {
    u64::try_from(value.as_nanos()).map_err(|_| CampaignError::Overflow)
}

fn new_id(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn validate_policy(policy: &PolicyManifest) -> Result<(), CampaignError> {
    duration(policy.host_compute_limit_s)?;
    if policy.policy_id.is_empty() || policy.backend.is_empty() || policy.num_reads == 0 {
        return Err(invalid(
            "policy_id, backend, and a positive num_reads are required",
        ));
    }
    if policy.preparation == (Preparation::Amortized { horizon: 0 }) {
        return Err(invalid("amortized preparation requires a positive horizon"));
    }
    Ok(())
}

fn encoding(instance: &Instance) -> Result<IsingEncoding, CampaignError> {
    match FixedIsing.encode(instance)? {
        EncodingOutcome::Exact(encoding) => Ok(encoding),
        EncodingOutcome::Surrogate(_) | EncodingOutcome::Unsupported { .. } => Err(invalid(
            "this runner requires the exact ising-fixed control",
        )),
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "Resource estimates are floating-point totals; counts remain exact u64 values."
)]
fn load_campaign(config: &CampaignConfig) -> Result<LoadedCampaign, CampaignError> {
    let manifest: Manifest = serde_json::from_reader(File::open(&config.manifest)?)?;
    if manifest.schema_version != 1 {
        return Err(CampaignError::UnsupportedVersion(manifest.schema_version));
    }
    let base = config.manifest.parent().unwrap_or_else(|| Path::new("."));
    let profile = load_protocol_profile(&base.join(manifest.protocol_profile))?;
    let budget = TrialBudget::new(
        manifest.budget.wall_limit_s.map(duration).transpose()?,
        manifest
            .budget
            .device_budget_s
            .map(|seconds| nanos(duration(seconds)?))
            .transpose()?,
    )?;
    let mut instance_ids = BTreeSet::new();
    let mut instances = Vec::new();
    for path in manifest.instances {
        let instance = load_instance(&base.join(path))?;
        encoding(&instance)?;
        target_from_objective(&instance)?;
        if !instance_ids.insert(instance.instance_id.clone()) {
            return Err(invalid("duplicate instance_id"));
        }
        instances.push(instance);
    }
    let mut policy_ids = BTreeSet::new();
    let mut policies = Vec::new();
    for policy in manifest.policies {
        validate_policy(&policy)?;
        if !policy_ids.insert(policy.policy_id.clone()) {
            return Err(invalid("duplicate policy_id"));
        }
        if policy.backend == config.backend {
            policies.push(policy);
        }
    }
    if instances.is_empty() || policies.is_empty() || manifest.trials_per_pair == 0 {
        return Err(invalid(
            "campaign requires instances, matching backend policies, and trials",
        ));
    }
    let trials = u64::try_from(instances.len())
        .map_err(|_| CampaignError::Overflow)?
        .checked_mul(u64::try_from(policies.len()).map_err(|_| CampaignError::Overflow)?)
        .and_then(|pairs| pairs.checked_mul(u64::from(manifest.trials_per_pair)))
        .ok_or(CampaignError::Overflow)?;
    let estimate = ResourceEstimate {
        trials,
        max_wall_s: manifest
            .budget
            .wall_limit_s
            .map(|seconds| seconds * trials as f64),
        max_device_s: manifest
            .budget
            .device_budget_s
            .map(|seconds| seconds * trials as f64),
        backends: vec![config.backend.clone()],
    };
    Ok(LoadedCampaign {
        instances,
        policies,
        trials_per_pair: manifest.trials_per_pair,
        budget,
        profile,
        estimate,
    })
}

fn resolve_backend(name: &str) -> BackendSpec {
    builtin_registry()
        .into_iter()
        .find(|backend| backend.name == name)
        .unwrap_or_else(|| BackendSpec {
            name: name.to_owned(),
            bin: PathBuf::from(name),
            args: Vec::new(),
            kind: BackendKind::Sweeping,
            node_capacity_flag: None,
        })
}

fn writable_directory(path: &Path) -> Result<(), CampaignError> {
    fs::create_dir_all(path).map_err(invalid)?;
    if fs::metadata(path)
        .map_err(invalid)?
        .permissions()
        .readonly()
    {
        return Err(invalid(format!(
            "output directory {} is read-only",
            path.display()
        )));
    }
    Ok(())
}

/// Validate the manifest, then run its matching policies or return a dry estimate.
///
/// # Errors
/// Returns input, accounting, transport-observation, or durable-output failures.
pub async fn run_campaign(config: &CampaignConfig) -> Result<CampaignReport, CampaignError> {
    run_campaign_with_backend(config, resolve_backend(&config.backend)).await
}

/// Run with an explicit backend, including the conformance fake miner.
///
/// # Errors
/// Same as [`run_campaign`]. Backend names must match the selected policies.
pub async fn run_campaign_with_backend(
    config: &CampaignConfig,
    backend: BackendSpec,
) -> Result<CampaignReport, CampaignError> {
    let loaded = load_campaign(config)?;
    let mut report = CampaignReport {
        resources: loaded.estimate,
        trials: Vec::new(),
    };
    if config.dry_run {
        return Ok(report);
    }
    if backend.name != config.backend {
        return Err(invalid("backend override name does not match the campaign"));
    }
    writable_directory(&config.output)?;
    let mut runner = TrialRunner::new(
        backend,
        config.output.clone(),
        RuleSet::Current(loaded.profile),
    );
    let result = async {
        for policy in &loaded.policies {
            // Preparation is shared within a policy, never across policy choices.
            runner.finish().await?;
            for instance in &loaded.instances {
                for _ in 0..loaded.trials_per_pair {
                    report
                        .trials
                        .push(runner.run_trial(instance, policy, &loaded.budget).await?);
                }
            }
        }
        runner.finish().await
    }
    .await;
    // Terminated trials remain available even when a later trial or shutdown fails.
    let mut file = File::create(config.output.join("campaign.json")).map_err(invalid)?;
    serde_json::to_writer(&mut file, &report).map_err(invalid)?;
    result?;
    Ok(report)
}

/// Run one cold or prepared trial using the bundled pinned protocol profile.
/// Traces are retained under `out/campaign`; use [`TrialRunner`] for explicit output and rules.
///
/// # Errors
/// Returns invalid inputs, arithmetic overflow, or an incomplete event trace.
pub async fn run_trial(
    instance: &Instance,
    policy: &PolicyManifest,
    budget: &TrialBudget,
) -> Result<Trial, CampaignError> {
    let profile = serde_json::from_str(include_str!(
        "../../evaluation/qblock/configs/protocol.json"
    ))?;
    let mut runner = TrialRunner::new(
        resolve_backend(&policy.backend),
        PathBuf::from("out/campaign"),
        RuleSet::Current(profile),
    );
    let trial = runner.run_trial(instance, policy, budget).await?;
    runner.finish().await?;
    Ok(trial)
}

struct LiveSession {
    id: String,
    commands: mpsc::Sender<SessionCommand>,
    events: mpsc::Receiver<SessionEvent>,
    failed: Arc<AtomicBool>,
    task: JoinHandle<Result<RunReport, SessionError>>,
    ready: bool,
    trials: u32,
}

impl LiveSession {
    fn start(backend: BackendSpec, spec: TopologySpec, id: String) -> Self {
        let socket = std::env::temp_dir().join(format!("{id}.sock"));
        let (commands, receiver) = mpsc::channel(4);
        let (sender, events) = mpsc::channel(32);
        let failed = Arc::new(AtomicBool::new(false));
        let control = SessionControl {
            commands: receiver,
            events: sender,
            failed: Arc::clone(&failed),
        };
        let task = tokio::spawn(async move {
            let backend = backend.with_node_capacity(spec.nodes.len());
            control::run_controlled(
                RunParams {
                    backend: &backend,
                    spec: &spec,
                    jobs: Vec::new(),
                    sock_path: &socket,
                    target: None,
                },
                control,
            )
            .await
        });
        Self {
            id,
            commands,
            events,
            failed,
            task,
            ready: false,
            trials: 0,
        }
    }

    async fn next(&mut self) -> Result<Option<SessionEvent>, CampaignError> {
        let event = self.events.recv().await;
        if self.failed.load(Ordering::Acquire) {
            return Err(invalid("session event channel overflowed or closed"));
        }
        if matches!(event, Some(SessionEvent::Ready)) {
            self.ready = true;
        }
        Ok(event)
    }
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Own a reusable miner session and the output/rule context absent from [`run_trial`].
pub struct TrialRunner {
    backend: BackendSpec,
    output: PathBuf,
    rules: RuleSet,
    run_id: String,
    session: Option<LiveSession>,
}

impl TrialRunner {
    #[must_use]
    pub fn new(backend: BackendSpec, output: PathBuf, rules: RuleSet) -> Self {
        Self {
            backend,
            output,
            rules,
            run_id: new_id("run"),
            session: None,
        }
    }

    /// End the prepared session and check for lost transport observations.
    ///
    /// # Errors
    /// Returns `InvalidRun` if the transport lost an event or its task panicked.
    pub async fn finish(&mut self) -> Result<(), CampaignError> {
        if let Some(mut session) = self.session.take() {
            let _ = session.commands.send(SessionCommand::Finish).await;
            let result = (&mut session.task).await.map_err(invalid)?;
            if session.failed.load(Ordering::Acquire) {
                return Err(invalid("session event delivery failed"));
            }
            let report = result.map_err(invalid)?;
            if !report.handshake_ok || report.miner_exit_code != 0 || report.miner_unresponsive {
                return Err(invalid("prepared miner session failed during shutdown"));
            }
        }
        Ok(())
    }

    /// Release a challenge, submit complete batches, and retain every trial event.
    ///
    /// # Errors
    /// Returns invalid inputs, accounting failures, or event writer failures.
    pub async fn run_trial(
        &mut self,
        instance: &Instance,
        policy: &PolicyManifest,
        budget: &TrialBudget,
    ) -> Result<Trial, CampaignError> {
        let result = self
            .run_cancellable(instance, policy, budget, None, None)
            .await;
        if result.is_err() {
            self.session.take();
        }
        result
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Keep preparation, release, and finalization in challenge-clock order."
    )]
    async fn run_cancellable(
        &mut self,
        instance: &Instance,
        policy: &PolicyManifest,
        budget: &TrialBudget,
        cancellation: Option<oneshot::Receiver<()>>,
        trial_id: Option<String>,
    ) -> Result<Trial, CampaignError> {
        validate_policy(policy)?;
        TrialBudget::new(budget.wall_limit, budget.device_budget_ns)?;
        if policy.backend != self.backend.name {
            return Err(invalid("trial backend does not match policy"));
        }
        if policy.preparation == Preparation::Cold
            || self
                .session
                .as_ref()
                .is_some_and(|session| match policy.preparation {
                    Preparation::Amortized { horizon } => session.trials >= horizon,
                    Preparation::Cold | Preparation::Warm => false,
                })
        {
            self.finish().await?;
        }
        let release_wall = SystemTime::now();
        let preparation_started = Instant::now();
        let encoding = encoding(instance)?;
        let target = target_from_objective(instance)?;
        let spec = topology(&encoding)?;
        let encoding_ns = nanos(preparation_started.elapsed())?;
        writable_directory(&self.output)?;
        let trial_id = trial_id.unwrap_or_else(|| new_id("trial"));
        let directory = self.output.join(&trial_id);
        fs::create_dir(&directory).map_err(invalid)?;
        let path = directory.join("events.jsonl");
        let writer = TraceWriter::create(&path).map_err(invalid)?;
        let mut state = TrialState::new(
            instance,
            policy,
            budget,
            trial_id,
            path,
            writer,
            self.run_id.clone(),
        );
        // The first prepared session starts outside challenge time. Its measured
        // preparation cost remains explicit, including amortization's horizon.
        state.count("host_compute_ns", encoding_ns)?;
        state.session_id = self
            .session
            .as_ref()
            .map_or_else(|| new_id("session"), |session| session.id.clone());
        if policy.preparation == Preparation::Cold {
            state.clock = preparation_started;
            state.trial.started = release_wall;
            state.release(instance, policy, budget)?;
        }
        if self.session.is_none() {
            self.session = Some(LiveSession::start(
                self.backend.clone(),
                spec.clone(),
                state.session_id.clone(),
            ));
        }
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| invalid("missing session"))?;
        let mut cancellation = cancellation;
        let mut preparation_terminal = None;
        if policy.preparation != Preparation::Cold && !session.ready {
            preparation_terminal = tokio::select! {
                biased;
                () = cancellation_requested(&mut cancellation) => Some(TerminalReason::Cancelled),
                result = tokio::time::timeout(PREPARATION_TIMEOUT, session.next()) => match result {
                    Ok(Ok(Some(SessionEvent::Ready))) => None,
                    Ok(Ok(_)) | Err(_) => Some(TerminalReason::MinerCrashed),
                    Ok(Err(error)) => return Err(error),
                },
            };
        }
        if policy.preparation != Preparation::Cold {
            state.trial.resources.insert(
                "preparation_ns".into(),
                nanos(preparation_started.elapsed())?,
            );
            state.clock = Instant::now();
            state.trial.started = SystemTime::now();
            if preparation_terminal.is_none() {
                state.release(instance, policy, budget)?;
            }
        }
        let terminal = if let Some(terminal) = preparation_terminal {
            Ok(terminal)
        } else {
            drive_trial(
                session,
                &mut state,
                instance,
                policy,
                &self.rules,
                &spec,
                &encoding,
                &target,
                &mut cancellation,
            )
            .await
        };
        let terminal = match terminal {
            Ok(terminal) => terminal,
            Err(error) => {
                // A write failure cannot be turned into a completed trial.
                self.session.take();
                return Err(error);
            }
        };
        session.trials = session.trials.saturating_add(1);
        if terminal != TerminalReason::Accepted || policy.preparation == Preparation::Cold {
            self.stop_trial(&mut state, terminal, instance, policy, &target)
                .await?;
        }
        state.terminate(terminal)?;
        Ok(state.trial)
    }

    async fn stop_trial(
        &mut self,
        state: &mut TrialState,
        terminal: TerminalReason,
        instance: &Instance,
        policy: &PolicyManifest,
        target: &quip_protocol::target::Target,
    ) -> Result<(), CampaignError> {
        let Some(mut session) = self.session.take() else {
            return Ok(());
        };
        if session.failed.load(Ordering::Acquire) {
            return Err(invalid("session event delivery failed"));
        }
        if session.ready {
            let _ = session.commands.send(SessionCommand::Finish).await;
        } else {
            session.task.abort();
            let _ = (&mut session.task).await;
            session.events.close();
        }
        while let Ok(event) = session.events.try_recv() {
            state.observe(event, instance, policy, &self.rules, target)?;
        }
        state.cancel_pending(terminal)?;
        if session.ready {
            while let Some(event) = session.next().await? {
                state.observe(event, instance, policy, &self.rules, target)?;
                state.cancel_pending(terminal)?;
            }
            let outcome = (&mut session.task).await.map_err(invalid)?;
            state.session_outcome = Some(match outcome {
                Ok(report) => {
                    json!({"handshake_ok": report.handshake_ok, "miner_exit_code": report.miner_exit_code,
                    "miner_unresponsive": report.miner_unresponsive, "orphan_results": report.orphan_results})
                }
                Err(error) => json!({"error": error.to_string()}),
            });
        }
        Ok(())
    }
}

fn topology(encoding: &IsingEncoding) -> Result<TopologySpec, CampaignError> {
    Ok(TopologySpec {
        nodes: (0..u32::try_from(encoding.h.len()).map_err(|_| CampaignError::Overflow)?).collect(),
        edges: encoding.edges.clone(),
        allowed_h_milli: encoding.h.clone(),
        allowed_j_milli: encoding.j.clone(),
        allowed_spin_milli: vec![-1000, 1000],
    })
}

fn submission(
    spec: &TopologySpec,
    encoding: &IsingEncoding,
    policy: &PolicyManifest,
    id: &str,
) -> Submission {
    let target = SetTarget {
        num_reads: policy.num_reads,
        num_sweeps: policy.num_sweeps,
        anneal_time_us: policy.anneal_time_us,
        ..SetTarget::default()
    };
    let job = Job {
        job_id: id.as_bytes().to_vec(),
        kind: JobKind::IsingSample as i32,
        ising: Some(IsingProblem {
            graph: Some(ising_problem::Graph::TopologyHash(spec.topology_hash())),
            encoding: CoefficientEncoding::I32 as i32,
            scale: crate::problem::MILLI_SCALE,
            h: encode_i32_le(&encoding.h),
            j: encode_i32_le(&encoding.j),
            num_reads: policy.num_reads,
            num_sweeps: policy.num_sweeps,
            anneal_time_us: policy.anneal_time_us,
            ..IsingProblem::default()
        }),
        ..Job::default()
    };
    Submission {
        job,
        spec: spec.clone(),
        target,
    }
}

struct TrialState {
    trial: Trial,
    clock: Instant,
    writer: TraceWriter,
    run_id: String,
    session_id: String,
    seq: u64,
    ledger: DeviceLedger,
    pending: BTreeSet<String>,
    cancelled: BTreeSet<String>,
    session_outcome: Option<Value>,
    witnesses: Vec<Witness>,
    energies: Vec<i64>,
}

impl TrialState {
    fn new(
        instance: &Instance,
        policy: &PolicyManifest,
        budget: &TrialBudget,
        trial_id: String,
        events: PathBuf,
        writer: TraceWriter,
        run_id: String,
    ) -> Self {
        Self {
            trial: Trial {
                trial_id,
                instance_id: instance.instance_id.clone(),
                policy_id: policy.policy_id.clone(),
                started: SystemTime::now(),
                wall_limit: budget.wall_limit,
                device_budget_ns: budget.device_budget_ns,
                resources: BTreeMap::new(),
                events,
                accepted_wall: None,
                device_total: DeviceTime::from_exact_ns(0),
                terminal: TerminalReason::InvalidRun,
                preparation: policy.preparation,
            },
            clock: Instant::now(),
            writer,
            run_id,
            session_id: String::new(),
            seq: 0,
            ledger: DeviceLedger::new(),
            pending: BTreeSet::new(),
            cancelled: BTreeSet::new(),
            session_outcome: None,
            witnesses: Vec::new(),
            energies: Vec::new(),
        }
    }

    fn release(
        &mut self,
        instance: &Instance,
        policy: &PolicyManifest,
        budget: &TrialBudget,
    ) -> Result<(), CampaignError> {
        self.emit(
            EventKind::ChallengeRelease,
            self.clock,
            None,
            json!({"instance": instance, "policy": policy,
            "wall_limit": budget.wall_limit, "device_budget_ns": budget.device_budget_ns}),
            None,
        )
    }

    fn emit(
        &mut self,
        kind: EventKind,
        at: Instant,
        job_id: Option<String>,
        payload: Value,
        device: Option<DeviceTime>,
    ) -> Result<(), CampaignError> {
        let event = TraceEvent {
            version: TRACE_VERSION,
            run_id: self.run_id.clone(),
            trial_id: self.trial.trial_id.clone(),
            session_id: self.session_id.clone(),
            seq: self.seq,
            observer: "campaign-controller".into(),
            kind,
            wall_ns: nanos(at.saturating_duration_since(self.clock))?,
            instance_id: Some(self.trial.instance_id.clone()),
            batch_id: (kind == EventKind::BatchReceipt)
                .then(|| job_id.clone())
                .flatten(),
            job_id,
            payload,
            device,
            anneal_ns: None,
            estimate_provenance: None,
            resources: self.trial.resources.clone(),
        };
        self.writer.append(&event).map_err(invalid)?;
        self.seq = self.seq.checked_add(1).ok_or(CampaignError::Overflow)?;
        Ok(())
    }

    fn count(&mut self, key: &str, amount: u64) -> Result<(), CampaignError> {
        let value = self.trial.resources.entry(key.to_owned()).or_default();
        *value = value.checked_add(amount).ok_or(CampaignError::Overflow)?;
        Ok(())
    }

    fn expired(&self) -> bool {
        self.trial
            .wall_limit
            .is_some_and(|limit| self.clock.elapsed() >= limit)
    }

    fn policy_terminal(
        &self,
        policy: &PolicyManifest,
    ) -> Result<Option<TerminalReason>, CampaignError> {
        if self
            .trial
            .resources
            .get("evaluations")
            .copied()
            .unwrap_or(0)
            >= policy.evaluation_limit
        {
            Ok(Some(TerminalReason::EvaluationLimit))
        } else if self.host_exhausted(policy)? {
            Ok(Some(TerminalReason::HostComputeLimit))
        } else {
            Ok(None)
        }
    }

    fn host_exhausted(&self, policy: &PolicyManifest) -> Result<bool, CampaignError> {
        Ok(Duration::from_nanos(
            self.trial
                .resources
                .get("host_compute_ns")
                .copied()
                .unwrap_or(0),
        ) >= duration(policy.host_compute_limit_s)?)
    }

    fn receive(&mut self, capture: &BatchCapture, at: Instant) -> Result<(), CampaignError> {
        let id = String::from_utf8(capture.job_id.clone()).map_err(invalid)?;
        if !self.pending.remove(&id) {
            return Err(invalid("capture has no matching dispatch"));
        }
        let cost = capture
            .meta
            .as_ref()
            .map_or_else(DeviceTime::missing, |meta| {
                DeviceTime::from_truncated_us(meta.qpu_access_us)
            });
        self.ledger.charge(&id, cost)?;
        self.count("batches", 1)?;
        self.emit(EventKind::BatchReceipt, at, Some(id), json!({
            "received_after_dispatch_ns": nanos(capture.received_after_dispatch)?,
            "solutions": capture.solutions.iter().map(|solution| json!({"spins": solution.spins,
                "reported_energy_milli": solution.reported_energy_milli,
                "recomputed_energy_milli": solution.recomputed_energy_milli})).collect::<Vec<_>>(),
            "meta": capture.meta.as_ref().map(|meta| json!({"reads": meta.reads, "sweeps": meta.sweeps,
                "device_access_time_us": meta.device_access_time_us, "qpu_access_us": meta.qpu_access_us, "extra": meta.extra}))
        }), Some(cost))
    }

    fn evaluate(
        &mut self,
        instance: &Instance,
        policy: &PolicyManifest,
        rules: &RuleSet,
        target: &quip_protocol::target::Target,
        capture: &BatchCapture,
    ) -> Result<(), CampaignError> {
        if self.trial.accepted_wall.is_some() {
            return Ok(());
        }
        for solution in &capture.solutions {
            if self.policy_terminal(policy)?.is_some() {
                break;
            }
            let witness = Witness {
                instance_id: instance.instance_id.clone(),
                spins: solution.spins.clone(),
                design: None,
                route: None,
                controls: None,
            };
            let started = Instant::now();
            let result = FixedIsing::evaluate_with_energy(instance, &witness);
            self.count("host_compute_ns", nanos(started.elapsed())?)?;
            self.count("evaluations", 1)?;
            let payload = match result {
                Ok((evaluation, energy)) => {
                    if evaluation.valid && evaluation.score.is_some() {
                        self.witnesses.push(witness.clone());
                        self.energies.push(energy);
                    }
                    json!({"witness": witness, "evaluation": evaluation})
                }
                Err(error) => json!({"witness": witness, "error": error}),
            };
            self.emit(
                EventKind::WitnessValidation,
                Instant::now(),
                Some(String::from_utf8_lossy(&capture.job_id).into_owned()),
                payload,
                None,
            )?;
        }
        if self.witnesses.is_empty() || self.host_exhausted(policy)? {
            return Ok(());
        }
        let started = Instant::now();
        let mut selected_indices = Vec::new();
        let decision = match rules {
            RuleSet::Current(profile) => {
                let reads: Vec<_> = self
                    .witnesses
                    .iter()
                    .zip(&self.energies)
                    .map(|(witness, &energy)| (witness.spins.as_slice(), energy))
                    .collect();
                match select_proof(&reads, target) {
                    Ok(indices) => {
                        selected_indices.clone_from(&indices);
                        let proof: Vec<_> = indices
                            .into_iter()
                            .map(|index| self.witnesses[index].clone())
                            .collect();
                        super::protocol::validate_set(instance, &proof, profile)?
                    }
                    Err(error) => ProofDecision::Rejected {
                        reason: error.to_string(),
                    },
                }
            }
            RuleSet::ProposedUnique { .. } => {
                selected_indices = (0..self.witnesses.len()).collect();
                rules.validate_set(instance, &self.witnesses)?
            }
        };
        self.count("host_compute_ns", nanos(started.elapsed())?)?;
        match decision {
            ProofDecision::Accepted { .. } => {
                let at = Instant::now();
                self.emit(
                    EventKind::ProofAcceptance,
                    at,
                    None,
                    json!({"decision": decision, "valid_witness_indices": selected_indices}),
                    None,
                )?;
                // Flush overhead belongs to the observed proof completion time.
                self.trial.accepted_wall = Some(self.clock.elapsed());
            }
            ProofDecision::Rejected { .. } => self.emit(
                EventKind::Rejection,
                Instant::now(),
                None,
                json!({"decision": decision}),
                None,
            )?,
        }
        Ok(())
    }

    fn cancel_pending(&mut self, terminal: TerminalReason) -> Result<(), CampaignError> {
        for id in self.pending.clone() {
            if !self.cancelled.insert(id.clone()) {
                continue;
            }
            self.ledger.launched_uncharged(&id)?;
            self.emit(
                EventKind::Cancellation,
                Instant::now(),
                Some(id),
                json!({"terminal": terminal}),
                Some(DeviceTime::missing()),
            )?;
        }
        Ok(())
    }

    fn observe(
        &mut self,
        event: SessionEvent,
        instance: &Instance,
        policy: &PolicyManifest,
        rules: &RuleSet,
        target: &quip_protocol::target::Target,
    ) -> Result<(), CampaignError> {
        match event {
            SessionEvent::Ready => Ok(()),
            SessionEvent::Dispatched { job_id, at } => self.dispatch(job_id, at),
            SessionEvent::Batch { capture, at } => {
                self.receive(&capture, at)?;
                self.evaluate(instance, policy, rules, target, &capture)
            }
            SessionEvent::Rejected { job_id, reason, at } => self.reject(job_id, &reason, at),
            SessionEvent::Failed { reason, at } => self.emit(
                EventKind::Rejection,
                at,
                None,
                json!({"session_failure": reason}),
                None,
            ),
        }
    }

    fn dispatch(&mut self, job_id: Vec<u8>, at: Instant) -> Result<(), CampaignError> {
        let id = String::from_utf8(job_id).map_err(invalid)?;
        self.ledger.launched_uncharged(&id)?;
        self.pending.insert(id.clone());
        self.count("jobs", 1)?;
        self.emit(EventKind::JobDispatch, at, Some(id), json!({}), None)
    }

    fn reject(&mut self, job_id: Vec<u8>, reason: &str, at: Instant) -> Result<(), CampaignError> {
        let id = String::from_utf8(job_id).map_err(invalid)?;
        self.pending.remove(&id);
        self.ledger.charge(&id, DeviceTime::missing())?;
        self.emit(
            EventKind::Rejection,
            at,
            Some(id),
            json!({"reason": reason}),
            Some(DeviceTime::missing()),
        )
    }

    fn terminate(&mut self, terminal: TerminalReason) -> Result<(), CampaignError> {
        self.cancel_pending(terminal)?;
        self.trial.device_total = self.ledger.total();
        if !self.pending.is_empty() {
            self.trial.device_total = self.trial.device_total.add(DeviceTime::missing())?;
        }
        self.trial.terminal = terminal;
        let device_outcome = self
            .trial
            .device_budget_ns
            .map(|limit| format!("{:?}", self.ledger.outcome(limit)));
        self.emit(
            EventKind::TrialTermination,
            Instant::now(),
            None,
            json!({"terminal": terminal,
            "accepted_wall": self.trial.accepted_wall, "device_outcome": device_outcome,
            "session_outcome": self.session_outcome}),
            Some(self.trial.device_total),
        )
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "The trial loop shares immutable problem and policy context with mutable trace and transport state."
)]
async fn drive_trial(
    session: &mut LiveSession,
    state: &mut TrialState,
    instance: &Instance,
    policy: &PolicyManifest,
    rules: &RuleSet,
    spec: &TopologySpec,
    encoding: &IsingEncoding,
    target: &quip_protocol::target::Target,
    cancellation: &mut Option<oneshot::Receiver<()>>,
) -> Result<TerminalReason, CampaignError> {
    loop {
        if state.expired() {
            return Ok(TerminalReason::WallTimeout);
        }
        if state
            .trial
            .device_budget_ns
            .is_some_and(|limit| state.ledger.outcome(limit) == BudgetOutcome::Exceeded)
        {
            return Ok(TerminalReason::DeviceBudgetExhausted);
        }
        if let Some(terminal) = state.policy_terminal(policy)? {
            return Ok(terminal);
        }
        let id = new_id("job");
        let mut submission = submission(spec, encoding, policy, &id);
        submission.target.max_energy_milli = target.max_energy_milli;
        submission.target.min_solutions = target.min_solutions;
        submission.target.min_diversity_milli = target.min_diversity_milli;
        if session
            .commands
            .send(SessionCommand::Submit(Box::new(submission)))
            .await
            .is_err()
        {
            return Ok(TerminalReason::MinerCrashed);
        }
        loop {
            let next = async {
                if let Some(limit) = state.trial.wall_limit {
                    tokio::time::timeout(
                        limit.saturating_sub(state.clock.elapsed()),
                        session.next(),
                    )
                    .await
                    .map_err(|_| TerminalReason::WallTimeout)
                } else {
                    Ok(session.next().await)
                }
            };
            let event = tokio::select! {
                biased;
                () = cancellation_requested(cancellation) => return Ok(TerminalReason::Cancelled),
                event = next => match event { Ok(event) => event?, Err(terminal) => return Ok(terminal) },
            };
            match event {
                Some(SessionEvent::Ready) => {}
                Some(SessionEvent::Dispatched { job_id, at }) => {
                    state.dispatch(job_id, at)?;
                }
                Some(SessionEvent::Batch { capture, at }) => {
                    state.receive(&capture, at)?;
                    state.evaluate(instance, policy, rules, target, &capture)?;
                    if state.trial.accepted_wall.is_some() {
                        return Ok(TerminalReason::Accepted);
                    }
                    if state.expired() {
                        return Ok(TerminalReason::WallTimeout);
                    }
                    if state
                        .trial
                        .device_budget_ns
                        .is_some_and(|limit| state.ledger.outcome(limit) == BudgetOutcome::Exceeded)
                    {
                        return Ok(TerminalReason::DeviceBudgetExhausted);
                    }
                    break;
                }
                Some(SessionEvent::Rejected { job_id, reason, at }) => {
                    state.reject(job_id, &reason, at)?;
                    return Ok(TerminalReason::MinerRejected);
                }
                Some(event @ SessionEvent::Failed { .. }) => {
                    state.observe(event, instance, policy, rules, target)?;
                    return Ok(TerminalReason::MinerCrashed);
                }
                None => return Ok(TerminalReason::MinerCrashed),
            }
        }
    }
}

async fn cancellation_requested(cancellation: &mut Option<oneshot::Receiver<()>>) {
    match cancellation {
        Some(receiver) => {
            let _ = receiver.await;
        }
        None => std::future::pending::<()>().await,
    }
}
