// SPDX-License-Identifier: AGPL-3.0-or-later
//! Backend registry: the built-in miner binaries this harness knows how to
//! drive, and capability/health probing for a resolved [`BackendSpec`].

mod registry;

pub use registry::{builtin_registry, probe_capabilities, probe_check, BackendSpec, ProbeError};
