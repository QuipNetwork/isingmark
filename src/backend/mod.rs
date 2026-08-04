// SPDX-License-Identifier: AGPL-3.0-or-later
//! Backend registry (the built-in miner binaries this harness knows how to
//! drive, and capability/health probing for a resolved [`BackendSpec`]) and
//! the coordinator-side session loop that drives one over
//! `MinerService.Session`.

mod registry;
mod session;

pub use registry::{builtin_registry, probe_capabilities, probe_check, BackendSpec, ProbeError};
pub use session::{run_session, RunParams, RunReport, SessionError};
