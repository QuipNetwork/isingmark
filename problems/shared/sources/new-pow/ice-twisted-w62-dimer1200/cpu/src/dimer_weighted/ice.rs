//! Alternating loops and boundary strings in a gauge-antiferromagnetic ice layer.

use super::{spin, threshold, Group, State, WeightedGraph};
use crate::dimer_research::IceLoopStats;
use rand::rngs::SmallRng;
use rand::Rng;
use std::collections::BTreeSet;

const BOUNDARY: usize = usize::MAX;

pub(super) struct IceTopology {
    tetrahedra: Vec<[usize; 4]>,
    membership: Vec<[usize; 2]>,
    gauge: Vec<[i32; 2]>,
    covered: Vec<usize>,
}

impl IceTopology {
    pub(super) fn new(graph: &WeightedGraph, groups: &[Group]) -> Result<Self, String> {
        if groups.is_empty() {
            return Err("ice moves require explicit tetrahedra".to_owned());
        }
        let mut membership = vec![[BOUNDARY; 2]; graph.neighbors.len()];
        let mut gauge = vec![[0; 2]; graph.neighbors.len()];
        let mut tetrahedra = Vec::with_capacity(groups.len());
        let mut strong_edges = BTreeSet::new();
        for (index, group) in groups.iter().enumerate() {
            if group.nodes.len() != 4 || group.internal.len() != 6 {
                return Err("ice groups must be complete four-spin tetrahedra".to_owned());
            }
            let mut local_gauge = [0; 4];
            local_gauge[0] = 1;
            for &(u, v, j) in &group.internal {
                let a = group.nodes[u];
                let b = group.nodes[v];
                if j.abs() != 1000 || !strong_edges.insert((a.min(b), a.max(b))) {
                    return Err("ice tetrahedra need distinct 1000-milli edges".to_owned());
                }
                if u == 0 {
                    local_gauge[v] = j.signum();
                } else if v == 0 {
                    local_gauge[u] = j.signum();
                }
            }
            if group
                .internal
                .iter()
                .any(|&(u, v, j)| local_gauge[u] * local_gauge[v] != j.signum())
            {
                return Err(
                    "strong signs are not a local gauge of antiferromagnetic ice".to_owned(),
                );
            }
            for (position, &node) in group.nodes.iter().enumerate() {
                let Some((slot_index, slot)) = membership[node]
                    .iter_mut()
                    .enumerate()
                    .find(|(_, slot)| **slot == BOUNDARY)
                else {
                    return Err("an ice spin belongs to at most two tetrahedra".to_owned());
                };
                *slot = index;
                gauge[node][slot_index] = local_gauge[position];
            }
            tetrahedra.push([
                group.nodes[0],
                group.nodes[1],
                group.nodes[2],
                group.nodes[3],
            ]);
        }
        // Extra strong bonds, including dimers, are part of the full-model
        // Metropolis delta. Only the explicit tetrahedra define loop topology.
        let covered: Vec<_> = membership
            .iter()
            .enumerate()
            .filter_map(|(node, members)| (members[0] != BOUNDARY).then_some(node))
            .collect();
        Ok(Self {
            tetrahedra,
            membership,
            gauge,
            covered,
        })
    }

    fn logical_spin(&self, state: &State, tetrahedron: usize, node: usize, lane: usize) -> i32 {
        let slot = usize::from(self.membership[node][0] != tetrahedron);
        debug_assert_eq!(self.membership[node][slot], tetrahedron);
        self.gauge[node][slot] * spin(state.spins[node], lane)
    }
}

enum MoveKind {
    Loop,
    String,
}

pub(super) struct IceWorkspace {
    visited: Vec<usize>,
    touched: Vec<usize>,
    marked: Vec<bool>,
    path: Vec<usize>,
    pub(super) stats: IceLoopStats,
}

impl IceWorkspace {
    pub(super) fn new(topology: &IceTopology) -> Self {
        Self {
            visited: vec![BOUNDARY; topology.tetrahedra.len()],
            touched: Vec::new(),
            marked: vec![false; topology.membership.len()],
            path: Vec::new(),
            stats: IceLoopStats::default(),
        }
    }

    fn propose(
        &mut self,
        topology: &IceTopology,
        state: &State,
        lane: usize,
        rng: &mut SmallRng,
    ) -> Option<MoveKind> {
        for &tetrahedron in &self.touched {
            self.visited[tetrahedron] = BOUNDARY;
        }
        self.touched.clear();
        self.path.clear();
        let start = topology.covered[rng.gen_range(0..topology.covered.len())];
        self.path.push(start);
        let members = topology.membership[start];
        let start_boundary = members[1] == BOUNDARY;
        let mut tetrahedron = if start_boundary {
            members[0]
        } else {
            members[rng.gen_range(0..2)]
        };
        loop {
            let incoming = self.path[self.path.len() - 1];
            let previous = self.visited[tetrahedron];
            if previous != BOUNDARY {
                let offset = previous + 1;
                if self.path.len() - offset < 2
                    || topology.logical_spin(state, tetrahedron, self.path[offset], lane)
                        == topology.logical_spin(state, tetrahedron, incoming, lane)
                {
                    return None;
                }
                self.path.drain(..offset);
                return Some(MoveKind::Loop);
            }
            let nodes = topology.tetrahedra[tetrahedron];
            let sum: i32 = nodes
                .iter()
                .map(|&node| topology.logical_spin(state, tetrahedron, node, lane))
                .sum();
            if sum != 0 {
                return None;
            }
            self.visited[tetrahedron] = self.path.len() - 1;
            self.touched.push(tetrahedron);
            let sign = topology.logical_spin(state, tetrahedron, incoming, lane);
            let mut opposite = [0; 2];
            let mut count = 0;
            for node in nodes {
                if topology.logical_spin(state, tetrahedron, node, lane) != sign {
                    opposite[count] = node;
                    count += 1;
                }
            }
            let outgoing = opposite[rng.gen_range(0..2)];
            self.path.push(outgoing);
            let pair = topology.membership[outgoing];
            let other = if pair[0] == tetrahedron {
                pair[1]
            } else {
                pair[0]
            };
            if other == BOUNDARY {
                return start_boundary.then_some(MoveKind::String);
            }
            tetrahedron = other;
        }
    }

    fn propose_defect(
        &mut self,
        topology: &IceTopology,
        state: &State,
        lane: usize,
        rng: &mut SmallRng,
    ) -> bool {
        for &tetrahedron in &self.touched {
            self.visited[tetrahedron] = BOUNDARY;
        }
        self.touched.clear();
        self.path.clear();
        let start = rng.gen_range(0..topology.tetrahedra.len());
        let charge: i32 = topology.tetrahedra[start]
            .iter()
            .map(|&node| topology.logical_spin(state, start, node, lane))
            .sum();
        if charge == 0 {
            return false;
        }
        let mut majority = [0; 4];
        let mut count = 0;
        for node in topology.tetrahedra[start] {
            if topology.logical_spin(state, start, node, lane) == charge.signum() {
                majority[count] = node;
                count += 1;
            }
        }
        let outgoing = majority[rng.gen_range(0..count)];
        self.path.push(outgoing);
        self.visited[start] = 0;
        self.touched.push(start);
        let mut previous = start;
        loop {
            let incoming = self.path[self.path.len() - 1];
            let pair = topology.membership[incoming];
            let tetrahedron = if pair[0] == previous {
                pair[1]
            } else {
                pair[0]
            };
            if tetrahedron == BOUNDARY {
                return self.path.len() >= 2;
            }
            if self.visited[tetrahedron] != BOUNDARY {
                return false;
            }
            let nodes = topology.tetrahedra[tetrahedron];
            let charge: i32 = nodes
                .iter()
                .map(|&node| topology.logical_spin(state, tetrahedron, node, lane))
                .sum();
            if charge != 0 || rng.gen_ratio(1, 8) {
                return self.path.len() >= 2;
            }
            self.visited[tetrahedron] = self.path.len() - 1;
            self.touched.push(tetrahedron);
            let sign = topology.logical_spin(state, tetrahedron, incoming, lane);
            let mut opposite = [0; 2];
            let mut count = 0;
            for node in nodes {
                if topology.logical_spin(state, tetrahedron, node, lane) != sign {
                    opposite[count] = node;
                    count += 1;
                }
            }
            debug_assert_eq!(count, 2);
            let outgoing = opposite[rng.gen_range(0..2)];
            if self.path.contains(&outgoing) {
                return false;
            }
            self.path.push(outgoing);
            previous = tetrahedron;
        }
    }

    fn half_delta(&mut self, graph: &WeightedGraph, state: &State, lane: usize) -> i32 {
        for &node in &self.path {
            self.marked[node] = true;
        }
        let mut delta = 0;
        for &node in &self.path {
            let sign = spin(state.spins[node], lane);
            delta -= sign * state.fields[node][lane];
            for &(other, j) in &graph.neighbors[node] {
                if node < other && self.marked[other] {
                    delta += 2 * j * sign * spin(state.spins[other], lane);
                }
            }
        }
        for &node in &self.path {
            self.marked[node] = false;
        }
        delta
    }

    fn apply_if_below(
        &mut self,
        graph: &WeightedGraph,
        state: &mut State,
        lane: usize,
        cut: i32,
    ) -> bool {
        if self.half_delta(graph, state, lane) > cut {
            return false;
        }
        for &node in &self.path {
            state.apply(graph, node, 1 << lane);
        }
        true
    }

    pub(super) fn defect_step(
        &mut self,
        topology: &IceTopology,
        graph: &WeightedGraph,
        state: &mut State,
        lane: usize,
        beta: f64,
        rng: &mut SmallRng,
    ) {
        self.stats.attempts += 1;
        self.stats.defect_attempts += 1;
        if !self.propose_defect(topology, state, lane, rng) {
            return;
        }
        self.stats.formed_defects += 1;
        let cut = threshold(rng, beta);
        if self.apply_if_below(graph, state, lane, cut) {
            self.stats.accepted += 1;
            self.stats.accepted_defects += 1;
            self.stats.flipped_spins += self.path.len() as u64;
        }
    }

    pub(super) fn step(
        &mut self,
        topology: &IceTopology,
        graph: &WeightedGraph,
        state: &mut State,
        lane: usize,
        beta: f64,
        rng: &mut SmallRng,
    ) {
        self.stats.attempts += 1;
        let Some(kind) = self.propose(topology, state, lane, rng) else {
            return;
        };
        match kind {
            MoveKind::Loop => self.stats.formed_loops += 1,
            MoveKind::String => self.stats.formed_strings += 1,
        }
        let cut = threshold(rng, beta);
        if self.apply_if_below(graph, state, lane, cut) {
            self.stats.accepted += 1;
            self.stats.flipped_spins += self.path.len() as u64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dimer_weighted::tests::{assert_cache, lane};
    use crate::dimer_weighted::{Group, State, WeightedGraph};
    use crate::sa_msc::{MscState, LANES};
    use quip_solver_core::IsingGraph;
    use rand::rngs::SmallRng;
    use rand::SeedableRng;

    fn fixture() -> (IsingGraph, Vec<Vec<usize>>, Vec<i8>) {
        let groups = vec![
            vec![0, 1, 4, 5],
            vec![1, 2, 6, 7],
            vec![2, 3, 8, 9],
            vec![3, 0, 10, 11],
        ];
        let gauge = [1, -1, -1, 1, -1, 1, 1, -1, 1, 1, -1, -1];
        let mut edges = Vec::new();
        let mut j = Vec::new();
        for group in &groups {
            for (i, &u) in group.iter().enumerate() {
                for &v in &group[i + 1..] {
                    edges.push((u, v));
                    j.push(f64::from(gauge[u] * gauge[v]));
                }
            }
        }
        for (u, v, weight) in [(4, 6, 0.125), (5, 10, -0.2), (0, 2, 0.125)] {
            edges.push((u, v));
            j.push(weight * f64::from(gauge[u] * gauge[v]));
        }
        let initial = gauge
            .iter()
            .enumerate()
            .map(|(i, &g)| if i % 2 == 0 { g } else { -g })
            .collect();
        (IsingGraph::new(vec![0.0; 12], j, edges), groups, initial)
    }

    fn locally_twisted_fixture() -> (IsingGraph, Vec<Vec<usize>>, Vec<i8>) {
        let groups = vec![
            vec![0, 1, 4, 5],
            vec![1, 2, 6, 7],
            vec![2, 3, 8, 9],
            vec![3, 0, 10, 11],
        ];
        let local_gauges = [
            [1, -1, -1, 1],
            [1, 1, -1, 1],
            [-1, 1, 1, 1],
            [1, -1, -1, -1],
        ];
        let mut edges = Vec::new();
        let mut j = Vec::new();
        for (group, gauge) in groups.iter().zip(local_gauges) {
            for (i, &u) in group.iter().enumerate() {
                for (k, &v) in group.iter().enumerate().skip(i + 1) {
                    edges.push((u, v));
                    j.push(f64::from(gauge[i] * gauge[k]));
                }
            }
        }
        for (u, v, weight) in [(4, 6, 0.125), (5, 10, -0.2), (0, 2, 0.125)] {
            edges.push((u, v));
            j.push(weight);
        }
        let initial = vec![1, 1, 1, 1, 1, 1, 1, -1, 1, -1, 1, -1];
        (IsingGraph::new(vec![0.0; 12], j, edges), groups, initial)
    }

    #[test]
    fn loops_and_boundary_strings_preserve_ice_and_match_full_energy() {
        let (base, nodes, initial) = fixture();
        let graph = WeightedGraph::new(&base).expect("milliweights");
        let groups = Group::build(&graph, &nodes, true).expect("tetrahedra");
        let topology = IceTopology::new(&graph, &groups).expect("gauged ice");
        let mut workspace = IceWorkspace::new(&topology);
        let mut rng = SmallRng::seed_from_u64(82);
        let seeds = vec![initial.as_slice(); LANES];
        let mut state = State::new(&graph, MscState::seeded(12, &seeds, &mut rng));
        let mut loops = 0;
        let mut strings = 0;
        let mut uphill = 0;
        let mut downhill = 0;
        for attempt in 0..2048 {
            let index = attempt % LANES;
            let Some(kind) = workspace.propose(&topology, &state, index, &mut rng) else {
                continue;
            };
            match kind {
                MoveKind::Loop => loops += 1,
                MoveKind::String => strings += 1,
            }
            let before = lane(&state, index);
            let mut after = before.clone();
            for &node in &workspace.path {
                after[node] *= -1;
            }
            let half_delta = workspace.half_delta(&graph, &state, index);
            let exact = graph.energy(&after) - graph.energy(&before);
            assert_eq!(2 * i64::from(half_delta), exact);
            uphill += usize::from(exact > 0);
            downhill += usize::from(exact < 0);
            let strong = |spins: &[i8]| -> i64 {
                graph
                    .edges
                    .iter()
                    .filter(|&&(_, _, j)| j.abs() == 1000)
                    .map(|&(u, v, j)| i64::from(j) * i64::from(spins[u]) * i64::from(spins[v]))
                    .sum()
            };
            assert_eq!(strong(&before), -8000);
            assert_eq!(strong(&after), -8000);
            assert!(!workspace.apply_if_below(&graph, &mut state, index, half_delta - 1));
            assert_eq!(lane(&state, index), before);
            assert!(workspace.apply_if_below(&graph, &mut state, index, half_delta));
            assert_eq!(lane(&state, index), after);
            assert_cache(&graph, &state);
        }
        assert!(loops > 0 && strings > 0 && uphill > 0 && downhill > 0);
    }

    #[test]
    fn locally_gauged_twist_forms_valid_boundary_strings() {
        let (base, nodes, initial) = locally_twisted_fixture();
        let global_cycle_sign: f64 = [(0, 1), (1, 2), (2, 3), (3, 0)]
            .iter()
            .map(|&(u, v)| {
                let edge = (u.min(v), u.max(v));
                let position = base
                    .edges
                    .iter()
                    .position(|&(a, b)| (a.min(b), a.max(b)) == edge)
                    .expect("strong cycle edge");
                base.j[position]
            })
            .product();
        assert_eq!(global_cycle_sign, -1.0);
        let graph = WeightedGraph::new(&base).expect("milliweights");
        let groups = Group::build(&graph, &nodes, true).expect("tetrahedra");
        let topology = IceTopology::new(&graph, &groups).expect("locally gauged ice");
        let mut workspace = IceWorkspace::new(&topology);
        let mut rng = SmallRng::seed_from_u64(82);
        let seeds = vec![initial.as_slice(); LANES];
        let mut state = State::new(&graph, MscState::seeded(12, &seeds, &mut rng));
        let mut strings = 0;
        let mut uphill = 0;
        let mut downhill = 0;
        for attempt in 0..2048 {
            let index = attempt % LANES;
            let Some(kind) = workspace.propose(&topology, &state, index, &mut rng) else {
                continue;
            };
            if let MoveKind::String = kind {
                strings += 1;
            }
            let before = lane(&state, index);
            let mut after = before.clone();
            for &node in &workspace.path {
                after[node] *= -1;
            }
            let half_delta = workspace.half_delta(&graph, &state, index);
            let exact = graph.energy(&after) - graph.energy(&before);
            assert_eq!(2 * i64::from(half_delta), exact);
            uphill += usize::from(exact > 0);
            downhill += usize::from(exact < 0);
            let strong = |spins: &[i8]| -> i64 {
                graph
                    .edges
                    .iter()
                    .filter(|&&(_, _, j)| j.abs() == 1000)
                    .map(|&(u, v, j)| i64::from(j) * i64::from(spins[u]) * i64::from(spins[v]))
                    .sum()
            };
            assert_eq!(strong(&before), -8000);
            assert_eq!(strong(&after), -8000);
            assert!(!workspace.apply_if_below(&graph, &mut state, index, half_delta - 1));
            assert_eq!(lane(&state, index), before);
            assert!(workspace.apply_if_below(&graph, &mut state, index, half_delta));
            assert_eq!(lane(&state, index), after);
            assert_cache(&graph, &state);
        }
        assert!(strings > 0 && uphill > 0 && downhill > 0);
    }

    #[test]
    fn extra_dimer_bonds_keep_exact_loop_energy() {
        let (base, nodes, initial) = locally_twisted_fixture();
        let mut edges = base.edges.to_vec();
        let mut couplings = base.j.clone();
        edges.push((4, 7));
        couplings.push(-1.0);
        let base = IsingGraph::new(base.h, couplings, edges);
        let graph = WeightedGraph::new(&base).expect("milliweights");
        let groups = Group::build(&graph, &nodes, true).expect("local blocks");
        let topology = IceTopology::new(&graph, &groups).expect("ice with extra dimer");
        let mut rng = SmallRng::seed_from_u64(47);
        let seeds = vec![initial.as_slice(); LANES];
        let mut state = State::new(&graph, MscState::seeded(12, &seeds, &mut rng));
        let mut workspace = IceWorkspace::new(&topology);
        for _ in 0..512 {
            workspace.step(&topology, &graph, &mut state, 0, 0.8, &mut rng);
            assert_cache(&graph, &state);
        }
        assert!(workspace.stats.accepted > 0);
    }

    #[test]
    fn ice_validation_rejects_non_ice_signs_and_shared_edges() {
        let (mut base, nodes, _) = fixture();
        base.j[0] *= -1.0;
        let graph = WeightedGraph::new(&base).expect("milliweights");
        let groups = Group::build(&graph, &nodes, true).expect("local blocks");
        assert!(IceTopology::new(&graph, &groups).is_err());
        let (base, mut nodes, _) = fixture();
        nodes.push(nodes[0].clone());
        let graph = WeightedGraph::new(&base).expect("milliweights");
        let groups = Group::build(&graph, &nodes, true).expect("local blocks");
        assert!(IceTopology::new(&graph, &groups).is_err());
    }

    #[test]
    fn public_ice_attack_replays_and_counts_only_requested_reads() {
        use crate::dimer_research::{sample_clusters, DimerVariant};
        use quip_solver_core::SampleParams;
        let (base, nodes, _) = locally_twisted_fixture();
        let params = SampleParams {
            num_reads: 65,
            num_sweeps: 8,
            sweeps_per_beta: 4,
            beta_range: Some((0.1, 10.0)),
            seed: 891,
        };
        let first = sample_clusters(&base, &params, DimerVariant::IceLoopWeighted, &nodes)
            .expect("ice solve");
        let second = sample_clusters(&base, &params, DimerVariant::IceLoopWeighted, &nodes)
            .expect("ice replay");
        assert_eq!(first.spins.len(), 65);
        assert_eq!(first.spins, second.spins);
        let stats = first.ice_loop_stats.expect("ice diagnostics");
        let replay = second.ice_loop_stats.expect("replay diagnostics");
        assert_eq!(
            (
                stats.attempts,
                stats.formed_loops,
                stats.formed_strings,
                stats.accepted,
                stats.flipped_spins,
            ),
            (
                replay.attempts,
                replay.formed_loops,
                replay.formed_strings,
                replay.accepted,
                replay.flipped_spins,
            )
        );
        assert_eq!(stats.attempts, 65 * 8);
        assert!(stats.accepted > 0);
        assert!(stats.accepted <= stats.formed_loops + stats.formed_strings);
        assert!(stats.formed_loops + stats.formed_strings <= stats.attempts);
        assert!(stats.flipped_spins >= 2 * stats.accepted);
    }

    #[test]
    fn dense_ice_attack_honors_the_64_attempt_budget() {
        use crate::dimer_research::{sample_clusters, DimerVariant};
        use quip_solver_core::SampleParams;
        let (base, nodes, _) = locally_twisted_fixture();
        let params = SampleParams {
            num_reads: 65,
            num_sweeps: 8,
            sweeps_per_beta: 4,
            beta_range: Some((0.1, 10.0)),
            seed: 891,
        };
        let result = sample_clusters(&base, &params, DimerVariant::IceLoop64Weighted, &nodes)
            .expect("dense ice solve");
        let replay = sample_clusters(&base, &params, DimerVariant::IceLoop64Weighted, &nodes)
            .expect("dense ice replay");
        assert_eq!(result.spins, replay.spins);
        let stats = result.ice_loop_stats.expect("dense ice diagnostics");
        let replay = replay.ice_loop_stats.expect("dense replay diagnostics");
        assert_eq!(
            (
                stats.attempts,
                stats.formed_loops,
                stats.formed_strings,
                stats.accepted,
                stats.flipped_spins,
            ),
            (
                replay.attempts,
                replay.formed_loops,
                replay.formed_strings,
                replay.accepted,
                replay.flipped_spins,
            )
        );
        assert_eq!(result.spins.len(), 65);
        assert_eq!(stats.attempts, 65 * 8 * 64);
        assert!(stats.formed_strings > 0 && stats.accepted > 0);
    }
    #[test]
    fn defect_paths_transport_and_annihilate_charge_with_exact_energy() {
        for (base, nodes, mut initial) in [fixture(), locally_twisted_fixture()] {
            initial[4] *= -1;
            let graph = WeightedGraph::new(&base).expect("milliweights");
            let groups = Group::build(&graph, &nodes, true).expect("tetrahedra");
            let topology = IceTopology::new(&graph, &groups).expect("ice topology");
            let mut workspace = IceWorkspace::new(&topology);
            let mut rng = SmallRng::seed_from_u64(2181);
            let seeds = vec![initial.as_slice(); LANES];
            let mut state = State::new(&graph, MscState::seeded(12, &seeds, &mut rng));
            let charges = |state: &State, lane: usize| -> Vec<i32> {
                topology
                    .tetrahedra
                    .iter()
                    .enumerate()
                    .map(|(tetrahedron, nodes)| {
                        nodes
                            .iter()
                            .map(|&node| topology.logical_spin(state, tetrahedron, node, lane))
                            .sum()
                    })
                    .collect()
            };
            let mut transported = 0;
            let mut annihilated = 0;
            for attempt in 0..4096 {
                let index = attempt % LANES;
                let before_charges = charges(&state, index);
                assert_eq!(before_charges.iter().filter(|&&q| q != 0).count(), 1);
                if !workspace.propose_defect(&topology, &state, index, &mut rng) {
                    continue;
                }
                assert!(workspace.path.len() >= 2);
                let unique: BTreeSet<_> = workspace.path.iter().copied().collect();
                assert_eq!(unique.len(), workspace.path.len());
                let before = lane(&state, index);
                let mut after = before.clone();
                for &node in &workspace.path {
                    after[node] *= -1;
                }
                let delta = workspace.half_delta(&graph, &state, index);
                assert_eq!(
                    2 * i64::from(delta),
                    graph.energy(&after) - graph.energy(&before)
                );
                assert!(!workspace.apply_if_below(&graph, &mut state, index, delta - 1));
                assert_eq!(lane(&state, index), before);
                assert!(workspace.apply_if_below(&graph, &mut state, index, delta));
                assert_eq!(lane(&state, index), after);
                assert_cache(&graph, &state);
                let after_charges = charges(&state, index);
                let defects = after_charges.iter().filter(|&&q| q != 0).count();
                assert!(defects <= 1);
                if defects == 0 {
                    annihilated += 1;
                }
                if defects == 1 && after_charges != before_charges {
                    transported += 1;
                }
                for other in 0..LANES {
                    if other != index {
                        assert_eq!(lane(&state, other), initial);
                    }
                }
                for &node in &workspace.path {
                    state.apply(&graph, node, 1 << index);
                }
                assert_eq!(lane(&state, index), initial);
                assert_cache(&graph, &state);
            }
            assert!(transported > 0, "no defect transport proposal");
            assert!(annihilated > 0, "no defect annihilation proposal");
        }
    }

    #[test]
    fn defect_paths_reject_a_satisfied_strong_layer() {
        let (base, nodes, initial) = locally_twisted_fixture();
        let graph = WeightedGraph::new(&base).expect("milliweights");
        let groups = Group::build(&graph, &nodes, true).expect("tetrahedra");
        let topology = IceTopology::new(&graph, &groups).expect("ice topology");
        let mut rng = SmallRng::seed_from_u64(31);
        let seeds = vec![initial.as_slice(); LANES];
        let state = State::new(&graph, MscState::seeded(12, &seeds, &mut rng));
        let mut workspace = IceWorkspace::new(&topology);
        for attempt in 0..256 {
            assert!(!workspace.propose_defect(&topology, &state, attempt % LANES, &mut rng));
        }
    }

    #[test]
    fn defect_paths_stop_at_a_second_charged_tetrahedron() {
        for (base, nodes, mut initial) in [fixture(), locally_twisted_fixture()] {
            initial[4] *= -1;
            initial[8] *= -1;
            let graph = WeightedGraph::new(&base).expect("milliweights");
            let groups = Group::build(&graph, &nodes, true).expect("tetrahedra");
            let topology = IceTopology::new(&graph, &groups).expect("ice");
            let mut rng = SmallRng::seed_from_u64(142);
            let seeds = vec![initial.as_slice(); LANES];
            let mut state = State::new(&graph, MscState::seeded(12, &seeds, &mut rng));
            let mut workspace = IceWorkspace::new(&topology);
            let charge = |state: &State, tetrahedron: usize| -> i32 {
                topology.tetrahedra[tetrahedron]
                    .iter()
                    .map(|&node| topology.logical_spin(state, tetrahedron, node, 0))
                    .sum()
            };
            let before_charges: Vec<_> = (0..4).map(|t| charge(&state, t)).collect();
            assert_eq!(before_charges.iter().filter(|&&q| q != 0).count(), 2);
            let mut charged_endpoints = 0;
            for _ in 0..4096 {
                if !workspace.propose_defect(&topology, &state, 0, &mut rng) {
                    continue;
                }
                let previous = *workspace.touched.last().expect("start tetrahedron");
                let pair = topology.membership[*workspace.path.last().expect("path")];
                let endpoint = if pair[0] == previous {
                    pair[1]
                } else {
                    pair[0]
                };
                if endpoint == BOUNDARY || charge(&state, endpoint) == 0 {
                    continue;
                }
                charged_endpoints += 1;
                assert!(!workspace.touched.contains(&endpoint));
                let start = workspace.touched[0];
                let mut after = initial.clone();
                for &node in &workspace.path {
                    after[node] *= -1;
                }
                let half_delta = workspace.half_delta(&graph, &state, 0);
                assert_eq!(
                    2 * i64::from(half_delta),
                    graph.energy(&after) - graph.energy(&initial)
                );
                assert!(workspace.apply_if_below(&graph, &mut state, 0, half_delta));
                assert_eq!(charge(&state, start), 0);
                for (t, &before) in before_charges.iter().enumerate() {
                    if t != start && t != endpoint {
                        assert_eq!(charge(&state, t), before);
                    }
                }
                assert_eq!(lane(&state, 0), after);
                assert_cache(&graph, &state);
                for &node in &workspace.path {
                    state.apply(&graph, node, 1);
                }
                assert_eq!(lane(&state, 0), initial);
                assert_cache(&graph, &state);
            }
            assert!(charged_endpoints > 0, "no charged endpoint proposal");
        }
    }

    #[test]
    fn defect_sampler_honors_attempt_budget_and_replays_partial_batches() {
        use crate::dimer_research::{sample_clusters, DimerVariant};
        use quip_solver_core::SampleParams;
        let (base, nodes, _) = locally_twisted_fixture();
        let params = SampleParams {
            num_reads: 65,
            num_sweeps: 8,
            sweeps_per_beta: 4,
            beta_range: Some((0.1, 10.0)),
            seed: 891,
        };
        let first = sample_clusters(&base, &params, DimerVariant::IceDefect64Weighted, &nodes)
            .expect("defect solve");
        let second = sample_clusters(&base, &params, DimerVariant::IceDefect64Weighted, &nodes)
            .expect("defect replay");
        assert_eq!(first.spins.len(), 65);
        assert_eq!(first.spins, second.spins);
        let stats = first.ice_loop_stats.expect("defect diagnostics");
        let replay = second.ice_loop_stats.expect("replay diagnostics");
        assert_eq!(
            serde_json::to_value(&stats).expect("stats"),
            serde_json::to_value(replay).expect("replay")
        );
        assert_eq!(stats.attempts, 65 * 8 * 64);
        assert_eq!(stats.defect_attempts, 65 * 8 * 32);
        assert!(stats.formed_defects <= stats.defect_attempts);
        assert!(stats.accepted_defects <= stats.formed_defects);
        assert!(stats.accepted_defects <= stats.accepted);
        assert!(stats.accepted <= stats.formed_loops + stats.formed_strings + stats.formed_defects);
        assert!(stats.flipped_spins >= 2 * stats.accepted);
    }

    #[test]
    fn original_loop_diagnostics_keep_their_serialized_fields() {
        let value = serde_json::to_value(IceLoopStats::default()).expect("stats JSON");
        let names: BTreeSet<_> = value
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            names,
            BTreeSet::from([
                "attempts",
                "formed_loops",
                "formed_strings",
                "accepted",
                "flipped_spins"
            ])
        );
    }
}
