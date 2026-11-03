// SPDX-License-Identifier: AGPL-3.0-or-later
//! Fixed solver portfolios: a declared solver catalog, panel slot resolution, and stage plans.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    path::{Path, PathBuf},
    time::{Instant, SystemTime},
};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::oneshot;

use super::{
    drive_trial, duration, encoding, invalid, nanos, new_id, resolve_backend, stop_session,
    topology, writable_directory, BudgetOutcome, CampaignError, EventKind, FamilyRegistry,
    Instance, InstanceIdentity, LiveSession, PolicyManifest, Preparation, RuleSet, TerminalReason,
    TraceWriter, Trial, TrialBudget, TrialState,
};
use crate::{backend::BackendSpec, campaign::protocol::target_from_objective};

/// The `--backend` value that selects a manifest's portfolio policies.
pub const PORTFOLIO_BACKEND: &str = "portfolio";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    Cpu,
    Gpu,
}

/// Which final states a solver returns for each job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateCapture {
    /// One state per read, including invalid and non-improving states.
    AllReads,
    /// Only the best state. Such a solver fails portfolio admission.
    BestOnly,
}

/// The families a solver's method applies to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppliesTo {
    Any,
    Families(Vec<String>),
}

impl AppliesTo {
    fn includes(&self, family: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Families(families) => families.iter().any(|known| known == family),
        }
    }
}

/// A registered solver binary and the capabilities it declares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolverDecl {
    pub solver_id: String,
    pub backend: String,
    pub device: Device,
    pub methods: BTreeSet<String>,
    pub applies_to: AppliesTo,
    pub state_capture: StateCapture,
    pub num_reads: u32,
    pub num_sweeps: u32,
    pub anneal_time_us: u32,
}

/// The share of the trial window that one stage may spend.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Allocation {
    /// At most this many seconds from the stage start, setup included.
    Seconds(f64),
    /// The rest of the trial window.
    Remainder,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleStage {
    pub slot: String,
    pub allocation: Allocation,
}

/// One panel slot: a single method, or a frozen schedule over other slots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    pub slot_id: String,
    pub device: Device,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// A generic method used only when no solver implements `method` for the family.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Vec<ScheduleStage>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Panel {
    pub panel_id: String,
    pub slots: Vec<Slot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub schema_version: u32,
    pub host_compute_limit_s: f64,
    pub evaluation_limit: u64,
    pub solvers: Vec<SolverDecl>,
    pub panels: Vec<Panel>,
}

/// Whether a stage runs the slot's own method or its declared generic fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Applicability {
    Native,
    Fallback,
}

/// One solver run inside a portfolio trial.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage {
    pub stage_id: String,
    pub solver_id: String,
    pub backend: String,
    pub method: String,
    pub applicability: Applicability,
    pub num_reads: u32,
    pub num_sweeps: u32,
    pub anneal_time_us: u32,
    pub allocation: Allocation,
}

/// A resolved slot: ordered stages that share one trial budget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortfolioPolicy {
    pub policy_id: String,
    pub host_compute_limit_s: f64,
    pub evaluation_limit: u64,
    pub stages: Vec<Stage>,
}

/// A solver that matched a slot's method but was not admitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejection {
    pub solver_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlotResolution {
    pub slot_id: String,
    /// `None` whenever any stage is missing, so an incomplete slot never runs.
    pub policy: Option<PortfolioPolicy>,
    pub missing: Vec<String>,
    pub rejected: Vec<Rejection>,
}

/// The dry-run view of one panel for one family. Resolution starts no solver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PanelResolution {
    pub panel_id: String,
    pub family: String,
    pub slots: Vec<SlotResolution>,
    pub missing_slots: Vec<String>,
    pub solver_starts: u64,
    pub complete: bool,
}

/// Load and check a solver catalog.
///
/// # Errors
/// Returns I/O, parse, version, or catalog consistency errors.
pub fn load_catalog(path: &Path) -> Result<Catalog, CampaignError> {
    let catalog: Catalog = serde_json::from_reader(File::open(path)?)?;
    if catalog.schema_version != 1 {
        return Err(CampaignError::UnsupportedVersion(catalog.schema_version));
    }
    duration(catalog.host_compute_limit_s)?;
    let mut solver_ids = BTreeSet::new();
    for solver in &catalog.solvers {
        if solver.solver_id.is_empty() || solver.backend.is_empty() || solver.num_reads == 0 {
            return Err(invalid(
                "solver_id, backend, and a positive num_reads are required",
            ));
        }
        if !solver_ids.insert(&solver.solver_id) {
            return Err(invalid(format!("duplicate solver {}", solver.solver_id)));
        }
    }
    let mut panel_ids = BTreeSet::new();
    for panel in &catalog.panels {
        if !panel_ids.insert(&panel.panel_id) {
            return Err(invalid(format!("duplicate panel {}", panel.panel_id)));
        }
        check_panel(panel)?;
    }
    Ok(catalog)
}

fn check_panel(panel: &Panel) -> Result<(), CampaignError> {
    let slots: BTreeMap<_, _> = panel
        .slots
        .iter()
        .map(|slot| (slot.slot_id.as_str(), slot))
        .collect();
    if slots.len() != panel.slots.len() {
        return Err(invalid(format!("{} repeats a slot id", panel.panel_id)));
    }
    for slot in &panel.slots {
        let context = format!("{}/{}", panel.panel_id, slot.slot_id);
        match (&slot.method, &slot.schedule) {
            (Some(_), None) => {}
            (None, Some(schedule)) => {
                if slot.fallback.is_some() {
                    return Err(invalid(format!("{context}: a schedule has no fallback")));
                }
                check_schedule(&context, slot.device, schedule, &slots)?;
            }
            _ => {
                return Err(invalid(format!(
                    "{context}: declare exactly one of method or schedule"
                )))
            }
        }
    }
    Ok(())
}

fn check_schedule(
    context: &str,
    device: Device,
    schedule: &[ScheduleStage],
    slots: &BTreeMap<&str, &Slot>,
) -> Result<(), CampaignError> {
    let Some((last, earlier)) = schedule.split_last() else {
        return Err(invalid(format!("{context}: empty schedule")));
    };
    if last.allocation != Allocation::Remainder {
        return Err(invalid(format!(
            "{context}: the last stage takes the remainder"
        )));
    }
    for stage in earlier {
        match stage.allocation {
            Allocation::Seconds(seconds) if seconds > 0.0 => {
                duration(seconds)?;
            }
            _ => {
                return Err(invalid(format!(
                    "{context}: earlier stages take positive seconds"
                )))
            }
        }
    }
    for stage in schedule {
        match slots.get(stage.slot.as_str()) {
            Some(target) if target.method.is_some() && target.device == device => {}
            _ => {
                return Err(invalid(format!(
                    "{context}: {} is not a method slot on the same device",
                    stage.slot
                )))
            }
        }
    }
    Ok(())
}

/// Only the last stage takes the remainder. Earlier stages take positive finite seconds.
fn check_allocations(context: &str, allocations: &[Allocation]) -> Result<(), CampaignError> {
    let Some((last, earlier)) = allocations.split_last() else {
        return Err(invalid(format!("{context}: no stages")));
    };
    if *last != Allocation::Remainder {
        return Err(invalid(format!(
            "{context}: the last stage takes the remainder"
        )));
    }
    for allocation in earlier {
        match *allocation {
            Allocation::Seconds(seconds) if seconds > 0.0 => {
                duration(seconds)?;
            }
            _ => {
                return Err(invalid(format!(
                    "{context}: earlier stages take positive seconds"
                )))
            }
        }
    }
    Ok(())
}

/// Check a resolved portfolio before any stage starts. Fixed allocations must
/// leave time for the remainder stage inside `wall_limit`.
///
/// # Errors
/// Returns `InvalidRun` for an empty or inconsistent portfolio.
pub fn validate_portfolio(
    portfolio: &PortfolioPolicy,
    budget: &TrialBudget,
) -> Result<(), CampaignError> {
    let context = &portfolio.policy_id;
    if context.is_empty() {
        return Err(invalid("a portfolio needs a policy_id"));
    }
    duration(portfolio.host_compute_limit_s)?;
    let allocations: Vec<_> = portfolio
        .stages
        .iter()
        .map(|stage| stage.allocation)
        .collect();
    check_allocations(context, &allocations)?;
    if portfolio
        .stages
        .iter()
        .any(|stage| stage.stage_id.is_empty() || stage.backend.is_empty() || stage.num_reads == 0)
    {
        return Err(invalid(format!(
            "{context}: every stage needs a stage_id, a backend, and a positive num_reads"
        )));
    }
    let fixed = allocations
        .iter()
        .try_fold(
            std::time::Duration::ZERO,
            |total, allocation| match *allocation {
                Allocation::Seconds(seconds) => Ok(total.saturating_add(duration(seconds)?)),
                Allocation::Remainder => Ok::<_, CampaignError>(total),
            },
        )?;
    if budget.wall_limit.is_some_and(|wall| fixed >= wall) {
        return Err(invalid(format!(
            "{context}: fixed allocations leave no time for the remainder stage"
        )));
    }
    Ok(())
}

fn stage_policy(portfolio: &PortfolioPolicy, stage: &Stage) -> PolicyManifest {
    // Stage limits are the parent's. The shared counters make them trial-wide.
    PolicyManifest {
        policy_id: portfolio.policy_id.clone(),
        backend: stage.backend.clone(),
        num_reads: stage.num_reads,
        num_sweeps: stage.num_sweeps,
        anneal_time_us: stage.anneal_time_us,
        host_compute_limit_s: portfolio.host_compute_limit_s,
        evaluation_limit: portfolio.evaluation_limit,
        preparation: Preparation::Cold,
    }
}

/// Run portfolio trials. Each stage starts a cold session on its own backend, and
/// every stage shares the trial clock, the device ledger, and the host counters.
pub struct PortfolioRunner {
    output: PathBuf,
    rules: RuleSet,
    registry: FamilyRegistry,
    run_id: String,
    backends: BTreeMap<String, BackendSpec>,
}

impl PortfolioRunner {
    #[must_use]
    pub fn new(output: PathBuf, rules: RuleSet) -> Self {
        Self {
            output,
            rules,
            registry: FamilyRegistry::production(),
            run_id: new_id("run"),
            backends: BTreeMap::new(),
        }
    }

    /// Resolve families through `registry` instead of the production registry.
    #[must_use]
    pub const fn with_registry(mut self, registry: FamilyRegistry) -> Self {
        self.registry = registry;
        self
    }

    /// Start `spec` for stages that name `spec.name`, instead of the registry entry.
    #[must_use]
    pub fn with_backend(mut self, spec: BackendSpec) -> Self {
        self.backends.insert(spec.name.clone(), spec);
        self
    }

    fn backend(&self, name: &str) -> BackendSpec {
        self.backends
            .get(name)
            .cloned()
            .unwrap_or_else(|| resolve_backend(name))
    }

    /// Run every stage of `portfolio` inside one trial and one budget.
    ///
    /// A stage that spends its allocation, crashes, or is rejected hands over to the
    /// next stage. Acceptance and the trial-wide limits end the trial.
    ///
    /// # Errors
    /// Returns invalid inputs, accounting failures, or event writer failures.
    #[expect(
        clippy::too_many_lines,
        reason = "Keep release, the stage sequence, and termination in challenge-clock order."
    )]
    pub async fn run_trial(
        &self,
        instance: &Instance,
        portfolio: &PortfolioPolicy,
        budget: &TrialBudget,
        mut cancellation: Option<oneshot::Receiver<()>>,
    ) -> Result<Trial, CampaignError> {
        validate_portfolio(portfolio, budget)?;
        TrialBudget::new(budget.wall_limit, budget.device_budget_ns)?;
        let release_wall = SystemTime::now();
        let clock = Instant::now();
        let encoding = encoding(&self.registry, instance)?;
        let identity = InstanceIdentity::new(instance, &encoding)?;
        let target = target_from_objective(instance)?;
        let spec = topology(&encoding)?;
        let encoding_ns = nanos(clock.elapsed())?;
        writable_directory(&self.output)?;
        let trial_id = new_id("trial");
        let directory = self.output.join(&trial_id);
        fs::create_dir(&directory).map_err(invalid)?;
        let path = directory.join("events.jsonl");
        let writer = TraceWriter::create(&path).map_err(invalid)?;
        let policies: Vec<PolicyManifest> = portfolio
            .stages
            .iter()
            .map(|stage| stage_policy(portfolio, stage))
            .collect();
        let first = policies.first().ok_or_else(|| invalid("no stages"))?;
        let mut state = TrialState::new(
            instance,
            first,
            budget,
            trial_id,
            path,
            writer,
            self.run_id.clone(),
            identity,
        );
        state.registry = self.registry;
        state.count("host_compute_ns", encoding_ns)?;
        state.clock = clock;
        state.trial.started = release_wall;
        state.session_id = new_id("session");
        state.release(instance, portfolio, budget)?;
        let mut terminal = TerminalReason::InvalidRun;
        for (index, (stage, policy)) in portfolio.stages.iter().zip(&policies).enumerate() {
            if let Some(stop) = limit_reached(&state, policy, &mut cancellation)? {
                terminal = stop;
                break;
            }
            if index > 0 {
                state.session_id = new_id("session");
            }
            state.session_outcome = None;
            let allocation_end = match stage.allocation {
                Allocation::Seconds(seconds) => {
                    Some(state.clock.elapsed().saturating_add(duration(seconds)?))
                }
                Allocation::Remainder => None,
            };
            let deadline = match (allocation_end, state.trial.wall_limit) {
                (Some(end), Some(wall)) => Some(end.min(wall)),
                (end, wall) => end.or(wall),
            };
            state.emit(
                EventKind::StageStart,
                Instant::now(),
                None,
                json!({"index": index, "stage": stage,
                    "deadline_ns": deadline.map(nanos).transpose()?}),
                None,
            )?;
            // Session setup runs inside the stage allocation.
            let mut session = LiveSession::start(
                self.backend(&stage.backend),
                spec.clone(),
                state.session_id.clone(),
            );
            let outcome = drive_trial(
                &mut session,
                &mut state,
                instance,
                policy,
                &self.rules,
                &spec,
                &encoding,
                &target,
                &mut cancellation,
                deadline,
            )
            .await?;
            let allocation_spent = outcome == TerminalReason::WallTimeout
                && allocation_end
                    .is_some_and(|end| state.trial.wall_limit.is_none_or(|wall| end < wall));
            let last = index + 1 == portfolio.stages.len();
            let hand_over = !last
                && match outcome {
                    TerminalReason::WallTimeout => allocation_spent,
                    TerminalReason::MinerCrashed | TerminalReason::MinerRejected => true,
                    _ => false,
                };
            if hand_over {
                // The next stage starts now. In-flight jobs are cancelled, not awaited.
                session.abandon();
            }
            stop_session(
                session,
                &mut state,
                outcome,
                instance,
                policy,
                &self.rules,
                &target,
            )
            .await?;
            state.emit(
                EventKind::StageEnd,
                Instant::now(),
                None,
                json!({"index": index, "stage_id": stage.stage_id,
                    "outcome": if allocation_spent { json!("allocation_spent") } else { json!(outcome) },
                    "session_outcome": state.session_outcome}),
                None,
            )?;
            terminal = outcome;
            if !hand_over {
                break;
            }
        }
        state.terminate(terminal)?;
        Ok(state.trial)
    }
}

/// A trial-wide limit or a cancellation that prevents the next stage from starting.
fn limit_reached(
    state: &TrialState,
    policy: &PolicyManifest,
    cancellation: &mut Option<oneshot::Receiver<()>>,
) -> Result<Option<TerminalReason>, CampaignError> {
    if cancellation.as_mut().is_some_and(|receiver| {
        !matches!(
            receiver.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        )
    }) {
        return Ok(Some(TerminalReason::Cancelled));
    }
    if state.expired() {
        return Ok(Some(TerminalReason::WallTimeout));
    }
    if state
        .trial
        .device_budget_ns
        .is_some_and(|limit| state.ledger.outcome(limit) == BudgetOutcome::Exceeded)
    {
        return Ok(Some(TerminalReason::DeviceBudgetExhausted));
    }
    state.policy_terminal(policy)
}

/// Whether `backend`'s binary exists on this host. Checks files only and starts nothing.
#[must_use]
pub fn backend_available(backend: &str) -> bool {
    let bin = super::resolve_backend(backend).bin;
    if bin.components().count() > 1 {
        return bin.is_file();
    }
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|directory| directory.join(&bin).is_file())
    })
}

/// Resolve every slot of `panel_id` for `family`. `available` reports whether a
/// backend binary can start on this host. Nothing starts during resolution.
///
/// # Errors
/// Returns `InvalidRun` for an unknown panel.
pub fn resolve_panel(
    catalog: &Catalog,
    panel_id: &str,
    family: &str,
    available: &dyn Fn(&str) -> bool,
) -> Result<PanelResolution, CampaignError> {
    let panel = catalog
        .panels
        .iter()
        .find(|panel| panel.panel_id == panel_id)
        .ok_or_else(|| invalid(format!("unknown panel {panel_id}")))?;
    let methods: BTreeMap<_, _> = panel
        .slots
        .iter()
        .filter_map(|slot| {
            slot.method.as_ref().map(|method| {
                let found = select(catalog, slot, method, family, available);
                (slot.slot_id.as_str(), (slot, found))
            })
        })
        .collect();
    let mut slots = Vec::new();
    for slot in &panel.slots {
        let plan: Vec<(&str, Allocation)> = match &slot.schedule {
            Some(schedule) => schedule
                .iter()
                .map(|stage| (stage.slot.as_str(), stage.allocation))
                .collect(),
            None => vec![(slot.slot_id.as_str(), Allocation::Remainder)],
        };
        let mut stages = Vec::new();
        let mut missing = Vec::new();
        let mut rejected = Vec::new();
        for (stage_slot, allocation) in plan {
            let Some((source, found)) = methods.get(stage_slot) else {
                return Err(invalid(format!(
                    "{panel_id}/{stage_slot} is not a method slot"
                )));
            };
            rejected.extend(found.rejected.iter().cloned());
            missing.extend(found.missing.iter().cloned());
            if let Some((solver, method, applicability)) = &found.chosen {
                stages.push(Stage {
                    stage_id: source.slot_id.clone(),
                    solver_id: solver.solver_id.clone(),
                    backend: solver.backend.clone(),
                    method: method.clone(),
                    applicability: *applicability,
                    num_reads: solver.num_reads,
                    num_sweeps: solver.num_sweeps,
                    anneal_time_us: solver.anneal_time_us,
                    allocation,
                });
            }
        }
        let policy = missing.is_empty().then(|| PortfolioPolicy {
            policy_id: format!("{panel_id}/{}", slot.slot_id),
            host_compute_limit_s: catalog.host_compute_limit_s,
            evaluation_limit: catalog.evaluation_limit,
            stages,
        });
        slots.push(SlotResolution {
            slot_id: slot.slot_id.clone(),
            policy,
            missing,
            rejected,
        });
    }
    let missing_slots: Vec<String> = slots
        .iter()
        .filter(|slot| slot.policy.is_none())
        .map(|slot| slot.slot_id.clone())
        .collect();
    Ok(PanelResolution {
        panel_id: panel_id.to_owned(),
        family: family.to_owned(),
        complete: missing_slots.is_empty(),
        slots,
        missing_slots,
        solver_starts: 0,
    })
}

struct Selection<'a> {
    chosen: Option<(&'a SolverDecl, String, Applicability)>,
    missing: Vec<String>,
    rejected: Vec<Rejection>,
}

/// Pick the first admissible solver by `solver_id` for the method, then for the fallback.
fn select<'a>(
    catalog: &'a Catalog,
    slot: &Slot,
    method: &str,
    family: &str,
    available: &dyn Fn(&str) -> bool,
) -> Selection<'a> {
    let mut selection = Selection {
        chosen: None,
        missing: Vec::new(),
        rejected: Vec::new(),
    };
    let mut solvers: Vec<&SolverDecl> = catalog.solvers.iter().collect();
    solvers.sort_by(|left, right| left.solver_id.cmp(&right.solver_id));
    let wanted = std::iter::once((method, Applicability::Native)).chain(
        slot.fallback
            .as_deref()
            .map(|fallback| (fallback, Applicability::Fallback)),
    );
    for (wanted, applicability) in wanted {
        for solver in &solvers {
            if solver.device != slot.device
                || !solver.methods.contains(wanted)
                || !solver.applies_to.includes(family)
            {
                continue;
            }
            if solver.state_capture == StateCapture::BestOnly {
                selection.rejected.push(Rejection {
                    solver_id: solver.solver_id.clone(),
                    reason: "returns only its best state".into(),
                });
            } else if !available(&solver.backend) {
                selection.rejected.push(Rejection {
                    solver_id: solver.solver_id.clone(),
                    reason: format!("backend {} is not available", solver.backend),
                });
            } else {
                selection.chosen = Some((solver, wanted.to_owned(), applicability));
                return selection;
            }
        }
    }
    selection.missing.push(format!(
        "{}: no admissible {} solver for {method}{} on {family}",
        slot.slot_id,
        match slot.device {
            Device::Cpu => "CPU",
            Device::Gpu => "GPU",
        },
        slot.fallback
            .as_deref()
            .map(|fallback| format!(" or fallback {fallback}"))
            .unwrap_or_default(),
    ));
    selection
}
