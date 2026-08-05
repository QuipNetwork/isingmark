// SPDX-License-Identifier: AGPL-3.0-or-later
//! Deterministic Ising problem draw over a topology spec.
//!
//! Mirrors `quip-coordinator`'s drive-mode `RandomSource`
//! (`crates/quip-coordinator/src/drive/random_source.rs`) and the
//! `derive_pow_job` it calls
//! (`crates/quip-coordinator/src/producer/pow.rs`): the same
//! `(seed, topology, index)` triple always derives the same nonce and the
//! same `PoW` job, via the network's own golden-pinned draw
//! (`quip_protocol::chacha8::draw_ising_milli`) and nonce derivation
//! (`quip_protocol::derive::derive_nonce`). In drive mode, upstream always
//! derives against a zero `last_proof_block_hash`
//! (`TopologySpec::to_snapshot` in
//! `crates/quip-coordinator/src/drive/topology_spec.rs`) — there is no chain
//! in benchmark mode to read a real one from — so this does the same.

use quip_proto::v1::{ising_problem, IsingProblem, Job, JobKind, Provenance};
use quip_protocol::chacha8::draw_ising_milli;
use quip_protocol::derive::derive_nonce;
use quip_protocol::wire::encode_i32_le;

use crate::topology::TopologySpec;

/// `last_proof_block_hash` upstream drive mode always derives against: there
/// is no chain in benchmark mode to read a real one from.
const BENCHMARK_LAST_PROOF: [u8; 32] = [0u8; 32];

/// Deterministic source of `count` `PoW` jobs over a topology, keyed by
/// `seed`: the same `(seed, spec)` pair always yields the same sequence of
/// jobs, in the same order, across instances and processes.
pub struct ProblemSource {
    spec: TopologySpec,
    miner_account: [u8; 32],
    seed: u64,
    count: u32,
    deadline_ms: u64,
    next_index: u32,
}

impl ProblemSource {
    /// Build a source that yields `count` deterministic jobs for
    /// `(spec, seed)`, each carrying `deadline_ms` as its wire deadline.
    #[must_use]
    pub fn new(
        spec: &TopologySpec,
        miner_account: [u8; 32],
        seed: u64,
        count: u32,
        deadline_ms: u64,
    ) -> Self {
        Self {
            spec: spec.clone(),
            miner_account,
            seed,
            count,
            deadline_ms,
            next_index: 0,
        }
    }

    /// Per-index salt: `blake3(seed_le || index_le)`, so a given
    /// `(seed, index)` always derives the same nonce and problem.
    #[must_use]
    pub fn salt_for(seed: u64, index: u32) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        let _ = h.update(&seed.to_le_bytes());
        let _ = h.update(&index.to_le_bytes());
        *h.finalize().as_bytes()
    }

    /// Draw the next job, or `None` once `count` jobs have been drawn.
    #[must_use]
    pub fn next_job(&mut self) -> Option<Job> {
        if self.next_index >= self.count {
            return None;
        }
        let salt = Self::salt_for(self.seed, self.next_index);
        let nonce = derive_nonce(BENCHMARK_LAST_PROOF, self.miner_account, salt);
        let generation = u64::from(self.next_index) + 1;
        // `draw_ising_milli` only errors when a required allowed-value set is
        // empty, but `TopologySpec` loading (`topology::load_topology`)
        // already rejects empty `allowed_h_milli`/`allowed_j_milli`
        // unconditionally, so this is unreachable for any `spec` this crate
        // can construct. Fail closed rather than unwrap.
        let (h, j) = draw_ising_milli(
            nonce,
            self.spec.nodes.len(),
            self.spec.edges.len(),
            &self.spec.allowed_h_milli,
            &self.spec.allowed_j_milli,
        )
        .ok()?;
        self.next_index += 1;
        Some(Job {
            job_id: nonce.to_vec(),
            kind: JobKind::IsingSample as i32,
            generation,
            deadline_ms: self.deadline_ms,
            ising: Some(IsingProblem {
                graph: Some(ising_problem::Graph::TopologyHash(
                    self.spec.topology_hash(),
                )),
                h_milli_le32: encode_i32_le(&h),
                j_milli_le32: encode_i32_le(&j),
                num_reads: 0,
                num_sweeps: 0,
                anneal_time_us: 0,
            }),
            provenance: Some(Provenance {
                is_pow: true,
                order_id: vec![],
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quip_protocol::wire::decode_i32_le;

    fn spec() -> TopologySpec {
        crate::topology::load_fixture("smoke").expect("smoke fixture parses")
    }

    /// A topology with a different node count than edge count, so a bug that
    /// swaps the `h`/`j` draw counts (or their allowed-value sets) changes
    /// the drawn length or value set rather than being masked by the smoke
    /// fixture's equal 8-node/8-edge shape.
    fn asymmetric_spec() -> TopologySpec {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("asymmetric.spec.json");
        std::fs::write(
            &path,
            r#"{
                "nodes": [0, 1, 2, 3, 4],
                "edges": [[0, 1], [1, 2], [2, 3]],
                "allowed_h_milli": [1000],
                "allowed_j_milli": [-2000, -1000]
            }"#,
        )
        .expect("write asymmetric fixture");
        crate::topology::load_topology(&path).expect("asymmetric fixture parses")
    }

    fn drain_all(src: &mut ProblemSource) -> Vec<Job> {
        let mut jobs = Vec::new();
        while let Some(job) = src.next_job() {
            jobs.push(job);
        }
        jobs
    }

    #[test]
    fn yields_exactly_count_jobs_then_none() {
        let spec = spec();
        let mut src = ProblemSource::new(&spec, [1u8; 32], 42, 5, 9_999_999);
        assert_eq!(drain_all(&mut src).len(), 5);
        assert!(src.next_job().is_none());
    }

    #[test]
    fn same_seed_and_topology_yields_identical_jobs_across_instances() {
        let spec = spec();
        let mut a = ProblemSource::new(&spec, [1u8; 32], 7, 3, 9_999_999);
        let mut b = ProblemSource::new(&spec, [1u8; 32], 7, 3, 9_999_999);
        let jobs_a = drain_all(&mut a);
        let jobs_b = drain_all(&mut b);
        assert_eq!(jobs_a.len(), jobs_b.len());
        for (ja, jb) in jobs_a.iter().zip(&jobs_b) {
            assert_eq!(ja.job_id, jb.job_id);
            assert_eq!(
                ja.ising
                    .as_ref()
                    .expect("job has an ising problem")
                    .h_milli_le32,
                jb.ising
                    .as_ref()
                    .expect("job has an ising problem")
                    .h_milli_le32
            );
        }
    }

    #[test]
    fn different_seeds_yield_different_job_ids() {
        let spec = spec();
        let mut a = ProblemSource::new(&spec, [1u8; 32], 1, 3, 9_999_999);
        let mut b = ProblemSource::new(&spec, [1u8; 32], 2, 3, 9_999_999);
        let id_a = a.next_job().expect("first job").job_id;
        let id_b = b.next_job().expect("first job").job_id;
        assert_ne!(id_a, id_b);
    }

    /// A job's id is its problem nonce, and the nonce is the sole entropy
    /// for the drawn couplings. If generation ignored the index, every
    /// problem in a run would be identical. The `salt_for` golden already
    /// encodes index mixing; this pins the user-facing consequence
    /// directly on consecutive `next_job` calls under a fixed seed.
    #[test]
    fn consecutive_jobs_under_fixed_seed_have_distinct_job_ids() {
        let spec = spec();
        let mut src = ProblemSource::new(&spec, [1u8; 32], 42, 3, 9_999_999);
        let id0 = src.next_job().expect("job 0").job_id;
        let id1 = src.next_job().expect("job 1").job_id;
        let id2 = src.next_job().expect("job 2").job_id;
        assert_ne!(id0, id1, "jobs 0 and 1 must differ under a fixed seed");
        assert_ne!(id1, id2, "jobs 1 and 2 must differ under a fixed seed");
        assert_ne!(id0, id2, "jobs 0 and 2 must differ under a fixed seed");
    }

    #[test]
    fn h_and_j_are_drawn_from_the_allowed_sets_at_the_right_lengths() {
        let spec = asymmetric_spec();
        let mut src = ProblemSource::new(&spec, [1u8; 32], 99, 4, 9_999_999);
        for job in drain_all(&mut src) {
            let ising = job.ising.expect("job has an ising problem");
            let h = decode_i32_le(&ising.h_milli_le32).expect("h decodes");
            let j = decode_i32_le(&ising.j_milli_le32).expect("j decodes");
            assert_eq!(h.len(), spec.nodes.len());
            assert_eq!(j.len(), spec.edges.len());
            assert!(h.iter().all(|v| spec.allowed_h_milli.contains(v)));
            assert!(j.iter().all(|v| spec.allowed_j_milli.contains(v)));
        }
    }

    #[test]
    fn salt_for_matches_blake3_of_seed_le_then_index_le() {
        let mut hasher = blake3::Hasher::new();
        let _ = hasher.update(&7u64.to_le_bytes());
        let _ = hasher.update(&3u32.to_le_bytes());
        let expected = *hasher.finalize().as_bytes();
        assert_eq!(ProblemSource::salt_for(7, 3), expected);
    }
}
