// SPDX-License-Identifier: AGPL-3.0-or-later
//! Backend registry (the built-in miner binaries this harness knows how to
//! drive, and capability/health probing for a resolved [`BackendSpec`]) and
//! the coordinator-side session loop that drives one over
//! `MinerService.Session`.

mod registry;
mod session;

pub use registry::{builtin_registry, probe_capabilities, probe_check, BackendSpec, ProbeError};
#[doc(hidden)]
pub use session::run_session_with_max_missed_pings_for_tests;
pub use session::{
    run_session, run_session_with_capture, BatchCapture, CapturedMeta, CapturedSolution, RunParams,
    RunReport, SessionError,
};
