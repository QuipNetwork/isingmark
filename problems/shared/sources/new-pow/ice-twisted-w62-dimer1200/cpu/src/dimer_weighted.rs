//! Integer-weight research kernels for dimers and frustrated blocks.

use quip_solver_core::{IsingGraph, SampleParams};
use rand::distributions::Open01;
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use std::time::Instant;

use crate::coloring::Coloring;
use crate::dimer_research::{contract, DimerSamples, DimerVariant, IceLoopStats};
use crate::research::validate_common;
use crate::sa_msc::{MscState, LANES};
use crate::sa_sampler::read_rng;
use crate::sampler_core::{build_beta_schedule, build_seeded_beta_schedule, CpuGraph};

mod ice;
mod paths;
pub(crate) mod tempering;

const SCALE: f64 = 1000.0;
const SUM_LIMIT: i64 = i32::MAX as i64 / 16;
const SINGLE_SALT: u64 = 0x5745_4947_4854_3031;
const GROUP_SALT: u64 = 0x4752_4f55_5053_3031;
const ICE_SALT: u64 = 0x4943_454c_4f4f_5031;
const MAX_BLOCK_SPINS: usize = 8;

struct WeightedGraph {
    neighbors: Vec<Vec<(usize, i32)>>,
    edges: Vec<(usize, usize, i32)>,
}

impl WeightedGraph {
    fn new(graph: &IsingGraph) -> Result<Self, String> {
        if graph.h.iter().any(|&h| h != 0.0) {
            return Err("weighted research kernels require zero fields".to_owned());
        }
        let mut neighbors = vec![Vec::new(); graph.h.len()];
        let mut edges = Vec::with_capacity(graph.edges.len());
        let mut total = 0i64;
        for (&(u, v), &j) in graph.edges.iter().zip(&graph.j) {
            let milli = j * SCALE;
            if !milli.is_finite()
                || (milli - milli.round()).abs() > 1e-8
                || milli.abs() > SUM_LIMIT as f64
            {
                return Err("couplings must be bounded exact milliweights".to_owned());
            }
            let weight = milli.round() as i32;
            total += i64::from(weight).abs();
            if total > SUM_LIMIT {
                return Err("sum of absolute milliweights exceeds i32 safety bound".to_owned());
            }
            neighbors[u].push((v, weight));
            neighbors[v].push((u, weight));
            edges.push((u, v, weight));
        }
        Ok(Self { neighbors, edges })
    }

    fn energy(&self, spins: &[i8]) -> i64 {
        self.edges
            .iter()
            .map(|&(u, v, j)| i64::from(j) * i64::from(spins[u]) * i64::from(spins[v]))
            .sum()
    }
}

struct Group {
    nodes: Vec<usize>,
    internal: Vec<(usize, usize, i32)>,
    energies: Vec<i32>,
}

impl Group {
    fn build(
        graph: &WeightedGraph,
        groups: &[Vec<usize>],
        local_blocks: bool,
    ) -> Result<Vec<Self>, String> {
        let mut membership = vec![None; graph.neighbors.len()];
        for (index, nodes) in groups.iter().enumerate() {
            if !(2..=MAX_BLOCK_SPINS).contains(&nodes.len()) {
                return Err("weighted clusters must have two through eight spins".to_owned());
            }
            for (position, &node) in nodes.iter().enumerate() {
                if node >= membership.len()
                    || nodes[..position].contains(&node)
                    || (!local_blocks && membership[node].is_some())
                {
                    return Err("invalid or repeated node in cluster definition".to_owned());
                }
                membership[node] = Some(index);
            }
        }
        for &(u, v, j) in &graph.edges {
            if !local_blocks
                && j.abs() == 1000
                && (membership[u].is_none() || membership[u] != membership[v])
            {
                return Err("clusters must include every strong-bond component".to_owned());
            }
        }
        let mut result = Vec::with_capacity(groups.len());
        for nodes in groups {
            let mut internal = Vec::new();
            for (i, &u) in nodes.iter().enumerate() {
                for &(v, j) in &graph.neighbors[u] {
                    if let Some(k) = nodes.iter().position(|&node| node == v) {
                        if i < k {
                            internal.push((i, k, j));
                        }
                    }
                }
            }
            let mut reached = vec![false; nodes.len()];
            reached[0] = true;
            for _ in 1..nodes.len() {
                for &(u, v, j) in &internal {
                    if j != 0 && (local_blocks || j.abs() == 1000) && (reached[u] || reached[v]) {
                        reached[u] = true;
                        reached[v] = true;
                    }
                }
            }
            if reached.iter().any(|&value| !value) {
                return Err("each cluster must be connected by its permitted bonds".to_owned());
            }
            let energies = (0..1usize << nodes.len())
                .map(|bits| {
                    internal
                        .iter()
                        .map(|&(u, v, j)| {
                            if ((bits >> u) ^ (bits >> v)) & 1 == 0 {
                                j
                            } else {
                                -j
                            }
                        })
                        .sum()
                })
                .collect();
            result.push(Self {
                nodes: nodes.clone(),
                internal,
                energies,
            });
        }
        Ok(result)
    }

    fn boundary(&self, state: &State) -> [[i32; LANES]; MAX_BLOCK_SPINS] {
        let mut boundary = [[0; LANES]; MAX_BLOCK_SPINS];
        for (fields, &node) in boundary.iter_mut().zip(&self.nodes) {
            *fields = state.fields[node];
        }
        for &(u, v, j) in &self.internal {
            let (left, right) = boundary.split_at_mut(v);
            for (lane, (field_u, field_v)) in left[u].iter_mut().zip(&mut right[0]).enumerate() {
                *field_u -= j * spin(state.spins[self.nodes[v]], lane);
                *field_v -= j * spin(state.spins[self.nodes[u]], lane);
            }
        }
        boundary
    }

    fn flip_mask(
        &self,
        state: &State,
        boundary: &[[i32; LANES]; MAX_BLOCK_SPINS],
        threshold: i32,
    ) -> u64 {
        let mut delta = [0; LANES];
        for (&node, fields) in self.nodes.iter().zip(boundary) {
            for (lane, value) in delta.iter_mut().enumerate() {
                *value -= spin(state.spins[node], lane) * fields[lane];
            }
        }
        let mut mask = 0;
        for (lane, &value) in delta.iter().enumerate() {
            if value <= threshold {
                mask |= 1 << lane;
            }
        }
        mask
    }

    fn minimize(
        &self,
        graph: &WeightedGraph,
        state: &mut State,
        boundary: &[[i32; LANES]; MAX_BLOCK_SPINS],
    ) {
        let mut best = [0; LANES];
        let mut current = [self.energies[0]; LANES];
        let mut best_words = [0; MAX_BLOCK_SPINS];
        for (i, &node) in self.nodes.iter().enumerate() {
            best_words[i] = state.spins[node];
        }
        for lane in 0..LANES {
            let mut bits = 0usize;
            for (i, &word) in best_words[..self.nodes.len()].iter().enumerate() {
                bits |= (((word >> lane) & 1) as usize) << i;
                best[lane] += spin(word, lane) * boundary[i][lane];
                current[lane] += boundary[i][lane];
            }
            best[lane] += self.energies[bits];
        }
        let mut previous = 0;
        for index in 0usize..self.energies.len() {
            let bits = index ^ (index >> 1);
            if index > 0 {
                let changed = index.trailing_zeros() as usize;
                let factor = if bits & (1 << changed) == 0 { 2 } else { -2 };
                let internal_delta = self.energies[bits] - self.energies[previous];
                for (value, &field) in current.iter_mut().zip(&boundary[changed]) {
                    *value += factor * field + internal_delta;
                }
            }
            let mut mask = 0;
            for (lane, (incumbent, &candidate)) in best.iter_mut().zip(&current).enumerate() {
                if candidate < *incumbent {
                    *incumbent = candidate;
                    mask |= 1 << lane;
                }
            }
            for (i, word) in best_words[..self.nodes.len()].iter_mut().enumerate() {
                if bits & (1 << i) == 0 {
                    *word &= !mask;
                } else {
                    *word |= mask;
                }
            }
            previous = bits;
        }
        for (&node, &word) in self.nodes.iter().zip(&best_words) {
            state.apply(graph, node, state.spins[node] ^ word);
        }
    }
}

#[inline]
fn spin(word: u64, lane: usize) -> i32 {
    1 - 2 * ((word >> lane) & 1) as i32
}

struct State {
    spins: Vec<u64>,
    fields: Vec<[i32; LANES]>,
}

impl State {
    fn new(graph: &WeightedGraph, packed: MscState) -> Self {
        let spins = packed.spins().to_vec();
        let mut fields = vec![[0; LANES]; spins.len()];
        for (u, neighbors) in graph.neighbors.iter().enumerate() {
            for &(v, j) in neighbors {
                for (lane, field) in fields[u].iter_mut().enumerate() {
                    *field += j * spin(spins[v], lane);
                }
            }
        }
        Self { spins, fields }
    }

    fn apply(&mut self, graph: &WeightedGraph, node: usize, mask: u64) {
        if mask == 0 {
            return;
        }
        let old = self.spins[node];
        self.spins[node] ^= mask;
        if mask.count_ones() < 16 {
            for &(neighbor, j) in &graph.neighbors[node] {
                let mut remaining = mask;
                while remaining != 0 {
                    let lane = remaining.trailing_zeros() as usize;
                    self.fields[neighbor][lane] -= 2 * j * spin(old, lane);
                    remaining &= remaining - 1;
                }
            }
        } else {
            let mut changes = [0; LANES];
            for (lane, change) in changes.iter_mut().enumerate() {
                *change = if mask & (1 << lane) == 0 {
                    0
                } else {
                    -2 * spin(old, lane)
                };
            }
            for &(neighbor, j) in &graph.neighbors[node] {
                for (field, &change) in self.fields[neighbor].iter_mut().zip(&changes) {
                    *field += j * change;
                }
            }
        }
    }

    fn single(&mut self, graph: &WeightedGraph, node: usize, threshold: i32) {
        let mut mask = 0;
        for (lane, &field) in self.fields[node].iter().enumerate() {
            if -spin(self.spins[node], lane) * field <= threshold {
                mask |= 1 << lane;
            }
        }
        self.apply(graph, node, mask);
    }
}

fn threshold(rng: &mut SmallRng, beta: f64) -> i32 {
    let u: f64 = rng.sample(Open01);
    (-u.ln() * SCALE / (2.0 * beta))
        .floor()
        .min(i32::MAX as f64) as i32
}

fn anneal(
    graph: &IsingGraph,
    params: &SampleParams,
    groups: &[Group],
    block: bool,
    seeds: Option<&[Vec<i8>]>,
    ice: Option<(&ice::IceTopology, usize, bool)>,
    path_blocks: bool,
) -> Result<(Vec<Vec<i8>>, IceLoopStats), String> {
    let weighted = WeightedGraph::new(graph)?;
    let mut paths = if path_blocks {
        Some(paths::Paths::new(&weighted)?)
    } else {
        None
    };
    let coloring = Coloring::new(&CpuGraph::from_base(graph));
    let mut states = Vec::new();
    for read in (0..params.num_reads).step_by(LANES) {
        let mut rng = read_rng(params.seed, read);
        let packed = match seeds {
            Some(seeds) => {
                let references: Vec<_> = seeds
                    .iter()
                    .skip(read)
                    .take(LANES)
                    .map(Vec::as_slice)
                    .collect();
                MscState::seeded(graph.h.len(), &references, &mut rng)
            }
            None => MscState::random(graph.h.len(), &mut rng),
        };
        states.push(State::new(&weighted, packed));
    }
    let betas = if seeds.is_some() {
        build_seeded_beta_schedule(graph, params, None)
    } else {
        build_beta_schedule(graph, params)
    };
    let mut single_rng = SmallRng::seed_from_u64(params.seed ^ SINGLE_SALT);
    let mut group_rng = SmallRng::seed_from_u64(params.seed ^ GROUP_SALT);
    let mut ice_rng = SmallRng::seed_from_u64(params.seed ^ ICE_SALT);
    let mut ice_work = ice.map(|(topology, attempts, defects)| {
        (
            topology,
            ice::IceWorkspace::new(topology),
            attempts,
            defects,
        )
    });
    for beta in betas {
        for _ in 0..params.sweeps_per_beta.max(1) {
            for class in coloring.classes() {
                for &node in class {
                    let cut = threshold(&mut single_rng, beta);
                    for state in &mut states {
                        state.single(&weighted, node as usize, cut);
                    }
                }
            }
            for group in groups {
                let cut = threshold(&mut group_rng, beta);
                for state in &mut states {
                    let boundary = group.boundary(state);
                    let mask = group.flip_mask(state, &boundary, cut);
                    for &node in &group.nodes {
                        state.apply(&weighted, node, mask);
                    }
                    // Flipping a block leaves its boundary spins unchanged.
                    if block {
                        group.minimize(&weighted, state, &boundary);
                    }
                }
            }
            if let Some(paths) = &mut paths {
                for state in &mut states {
                    paths.minimize(&weighted, state);
                }
            }
            if let Some((topology, workspace, attempts, defects)) = &mut ice_work {
                for (chunk, state) in states.iter_mut().enumerate() {
                    for lane in 0..LANES.min(params.num_reads - chunk * LANES) {
                        for attempt in 0..*attempts {
                            if *defects && attempt % 2 == 1 {
                                workspace.defect_step(
                                    topology,
                                    &weighted,
                                    state,
                                    lane,
                                    beta,
                                    &mut ice_rng,
                                );
                            } else {
                                workspace.step(
                                    topology,
                                    &weighted,
                                    state,
                                    lane,
                                    beta,
                                    &mut ice_rng,
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    let spins = (0..params.num_reads)
        .map(|read| {
            states[read / LANES]
                .spins
                .iter()
                .map(|&word| spin(word, read % LANES) as i8)
                .collect()
        })
        .collect();
    let stats = ice_work
        .map(|(_, workspace, _, _)| workspace.stats)
        .unwrap_or_default();
    Ok((spins, stats))
}

pub(crate) fn sample(
    graph: &IsingGraph,
    params: &SampleParams,
    variant: DimerVariant,
    groups: &[Vec<usize>],
) -> Result<DimerSamples, String> {
    let started = Instant::now();
    validate_common(graph, params, None).map_err(|error| error.to_string())?;
    if params.beta_range.is_none() {
        return Err("an explicit beta range is required".to_owned());
    }
    let weighted = WeightedGraph::new(graph)?;
    let moves = Group::build(
        &weighted,
        groups,
        variant == DimerVariant::LocalBlockWeighted
            || variant == DimerVariant::PathBlockWeighted
            || variant == DimerVariant::IceLoopWeighted
            || variant == DimerVariant::IceLoop64Weighted
            || variant == DimerVariant::IceDefect64Weighted,
    )?;
    let ice = if variant == DimerVariant::IceLoopWeighted
        || variant == DimerVariant::IceLoop64Weighted
        || variant == DimerVariant::IceDefect64Weighted
    {
        Some(ice::IceTopology::new(&weighted, &moves)?)
    } else {
        None
    };
    let ice_attempts = if variant == DimerVariant::IceLoop64Weighted
        || variant == DimerVariant::IceDefect64Weighted
    {
        64
    } else {
        1
    };
    let (collective, block, reduced, polish) = match variant {
        DimerVariant::BaselineWeighted => (false, false, false, false),
        DimerVariant::PairWeighted => {
            if groups.iter().any(|nodes| nodes.len() != 2) {
                return Err("pair moves require dimers".to_owned());
            }
            (true, false, false, false)
        }
        DimerVariant::ClusterWeighted => (true, false, false, false),
        DimerVariant::BlockWeighted
        | DimerVariant::LocalBlockWeighted
        | DimerVariant::PathBlockWeighted
        | DimerVariant::IceLoopWeighted
        | DimerVariant::IceLoop64Weighted
        | DimerVariant::IceDefect64Weighted => (true, true, false, false),
        DimerVariant::ReducedWeighted => (false, false, true, false),
        DimerVariant::ReducedPolishWeighted => (false, false, true, true),
        DimerVariant::PtIceFullWeighted
        | DimerVariant::PtIceStrongWeighted
        | DimerVariant::PtIceGeometricFullWeighted
        | DimerVariant::PtIceGeometricStrongWeighted
        | DimerVariant::PtIceProfileFullWeighted
        | DimerVariant::PtIceProfileStrongWeighted
        | DimerVariant::PtIceBandFullWeighted
        | DimerVariant::PtIceBandStrongWeighted => {
            return Err("tempering variants require the tempering sampler".to_owned());
        }
        DimerVariant::BaselineUnit
        | DimerVariant::PairUnit
        | DimerVariant::ClusterUnit
        | DimerVariant::ReducedUnit
        | DimerVariant::ReducedPolishUnit => {
            return Err("unit variants require the unit sampler".to_owned());
        }
    };
    if reduced
        && (groups.iter().any(|nodes| nodes.len() != 2)
            || moves
                .iter()
                .any(|group| group.internal.iter().any(|&(_, _, j)| j != -1000)))
    {
        return Err("contraction requires ferromagnetic dimers".to_owned());
    }
    let polish_sweeps = if polish { params.num_sweeps / 8 } else { 0 };
    let solve_sweeps = params.num_sweeps - polish_sweeps;
    let per_beta = params.sweeps_per_beta.max(1);
    if !solve_sweeps.is_multiple_of(per_beta)
        || (polish
            && (!params.num_sweeps.is_multiple_of(8)
                || polish_sweeps == 0
                || !polish_sweeps.is_multiple_of(per_beta)))
    {
        return Err(
            "stage budgets must contain whole beta blocks and polish uses 7:1 sweeps".to_owned(),
        );
    }
    let solve_params = SampleParams {
        num_sweeps: solve_sweeps,
        ..params.clone()
    };
    let (mut spins, ice_loop_stats) = if reduced {
        let (contracted, sites, _) = contract(graph, groups);
        let (states, _) = anneal(&contracted, &solve_params, &[], false, None, None, false)?;
        (
            states
                .iter()
                .map(|state| sites.iter().map(|&site| state[site]).collect())
                .collect(),
            None,
        )
    } else {
        let (states, stats) = anneal(
            graph,
            &solve_params,
            if collective { &moves } else { &[] },
            block,
            None,
            ice.as_ref().map(|topology| {
                (
                    topology,
                    ice_attempts,
                    variant == DimerVariant::IceDefect64Weighted,
                )
            }),
            variant == DimerVariant::PathBlockWeighted,
        )?;
        (states, ice.as_ref().map(|_| stats))
    };
    let solve_seconds = started.elapsed().as_secs_f64();
    let mut expanded_spins = None;
    let mut final_spins = None;
    let mut polish_seconds = 0.0;
    if polish {
        let started = Instant::now();
        let params = SampleParams {
            num_sweeps: polish_sweeps,
            seed: params.seed ^ 0x504F_4C49_5348_3031,
            ..params.clone()
        };
        let (final_states, _) = anneal(graph, &params, &[], false, Some(&spins), None, false)?;
        expanded_spins = Some(spins.clone());
        for (incumbent, candidate) in spins.iter_mut().zip(&final_states) {
            if weighted.energy(candidate) < weighted.energy(incumbent) {
                incumbent.clone_from(candidate);
            }
        }
        final_spins = Some(final_states);
        polish_seconds = started.elapsed().as_secs_f64();
    }
    Ok(DimerSamples {
        spins,
        expanded_spins,
        final_spins,
        solve_seconds,
        polish_seconds,
        solve_sweeps,
        polish_sweeps,
        ice_loop_stats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dimer_research::sample_clusters;
    use crate::sa_msc_f64::energy_f64;

    fn fixture(k: usize, outside: usize) -> (IsingGraph, Vec<Vec<usize>>) {
        let mut edges = Vec::new();
        let mut couplings = Vec::new();
        for u in 0..k {
            for v in u + 1..k {
                edges.push((u, v));
                couplings.push(if (u + v) % 3 == 0 { 1.0 } else { -1.0 });
            }
            for v in k..k + outside {
                edges.push((u, v));
                let magnitude = ((17 * u + 31 * v) % 125 + 1) as f64 / 1000.0;
                couplings.push(if v % 2 == 0 { magnitude } else { -magnitude });
            }
        }
        (
            IsingGraph::new(vec![0.0; k + outside], couplings, edges),
            vec![(0..k).collect()],
        )
    }

    fn packed_fixture(graph: &WeightedGraph) -> State {
        let mut rng = SmallRng::seed_from_u64(891);
        State::new(graph, MscState::random(graph.neighbors.len(), &mut rng))
    }

    pub(super) fn lane(state: &State, index: usize) -> Vec<i8> {
        state
            .spins
            .iter()
            .map(|&word| spin(word, index) as i8)
            .collect()
    }

    pub(super) fn assert_cache(graph: &WeightedGraph, state: &State) {
        for (node, neighbors) in graph.neighbors.iter().enumerate() {
            for lane in 0..LANES {
                let direct: i32 = neighbors
                    .iter()
                    .map(|&(other, j)| j * spin(state.spins[other], lane))
                    .sum();
                assert_eq!(state.fields[node][lane], direct, "node {node}, lane {lane}");
            }
        }
    }

    #[test]
    fn weighted_group_deltas_exclude_all_internal_bonds_at_large_degree() {
        for k in [2, 4, 6] {
            let (base, groups) = fixture(k, 140);
            let graph = WeightedGraph::new(&base).expect("milliweights");
            let group = Group::build(&graph, &groups, false)
                .expect("signed components")
                .remove(0);
            let mut state = packed_fixture(&graph);
            let boundary = group.boundary(&state);
            for threshold in [0, 1, 37, 125, 1000, 8000, i32::MAX] {
                let mask = group.flip_mask(&state, &boundary, threshold);
                for index in 0..LANES {
                    let before = lane(&state, index);
                    let mut after = before.clone();
                    for &node in &group.nodes {
                        after[node] *= -1;
                    }
                    let delta = ((energy_f64(&after, &base) - energy_f64(&before, &base)) * 1000.0)
                        .round() as i64;
                    assert_eq!((mask >> index) & 1 == 1, delta <= 2 * i64::from(threshold));
                }
            }
            for &node in &group.nodes {
                state.apply(&graph, node, 0xf0f0_0000_ffff_0123);
            }
            assert_cache(&graph, &state);
        }
    }

    #[test]
    fn exact_blocks_match_exhaustive_boundary_conditioned_energies() {
        for k in [2, 4, 6] {
            let (base, groups) = fixture(k, 3);
            let graph = WeightedGraph::new(&base).expect("milliweights");
            let group = Group::build(&graph, &groups, false)
                .expect("signed groups")
                .remove(0);
            let mut state = packed_fixture(&graph);
            let before: Vec<_> = (0..LANES).map(|index| lane(&state, index)).collect();
            let boundary = group.boundary(&state);
            group.minimize(&graph, &mut state, &boundary);
            for (index, initial) in before.iter().enumerate() {
                let optimum = (0usize..1 << k)
                    .map(|bits| {
                        let mut candidate = initial.clone();
                        for (node, s) in candidate.iter_mut().take(k).enumerate() {
                            *s = if bits & (1 << node) == 0 { 1 } else { -1 };
                        }
                        (energy_f64(&candidate, &base) * 1000.0).round() as i64
                    })
                    .min()
                    .expect("nonempty enumeration");
                let actual = lane(&state, index);
                assert_eq!(
                    (energy_f64(&actual, &base) * 1000.0).round() as i64,
                    optimum
                );
                assert_eq!(&actual[k..], &initial[k..]);
            }
            assert_cache(&graph, &state);
        }
    }

    #[test]
    fn eight_spin_cell_update_reaches_the_exact_conditional_minimum() {
        let mut edges = Vec::new();
        let mut weights = Vec::new();
        for u in 0..4 {
            for v in 4..8 {
                edges.push((u, v));
                weights.push(if (u + v) % 2 == 0 { -0.143 } else { 0.143 });
            }
        }
        for u in 0..8 {
            edges.push((u, 8 + u));
            weights.push(if u % 3 == 0 { -1.0 } else { 1.0 });
        }
        let base = IsingGraph::new(vec![0.0; 16], weights, edges);
        let graph = WeightedGraph::new(&base).expect("milliweights");
        let groups = Group::build(&graph, &[(0..8).collect()], true).expect("eight-spin cell");
        let group = &groups[0];
        let mut state = packed_fixture(&graph);
        let before: Vec<_> = (0..LANES).map(|index| lane(&state, index)).collect();
        let boundary = group.boundary(&state);
        group.minimize(&graph, &mut state, &boundary);
        for (index, initial) in before.iter().enumerate() {
            let optimum = (0..256)
                .map(|bits| {
                    let mut trial = initial.clone();
                    for (node, value) in trial.iter_mut().enumerate().take(8) {
                        *value = if bits & (1 << node) == 0 { 1 } else { -1 };
                    }
                    (energy_f64(&trial, &base) * 1000.0).round() as i64
                })
                .min()
                .expect("all cell states");
            let actual = lane(&state, index);
            assert_eq!(
                (energy_f64(&actual, &base) * 1000.0).round() as i64,
                optimum
            );
            assert_eq!(&actual[8..], &initial[8..]);
        }
        assert_cache(&graph, &state);
        let (wide, nodes) = fixture(9, 0);
        assert!(Group::build(&WeightedGraph::new(&wide).expect("graph"), &nodes, true).is_err());
    }

    #[test]
    fn overlapping_local_blocks_match_exhaustive_minima_and_preserve_cached_fields() {
        let base = IsingGraph::new(
            vec![0.0; 5],
            vec![1.0, 1.0, 1.0, 1.0, -0.2, 0.125],
            vec![(0, 1), (1, 2), (0, 2), (2, 3), (3, 4), (0, 4)],
        );
        let graph = WeightedGraph::new(&base).expect("milliweights");
        let nodes = vec![vec![0, 1, 2], vec![2, 3, 4]];
        assert!(Group::build(&graph, &nodes, false).is_err());
        let groups = Group::build(&graph, &nodes, true).expect("overlapping local blocks");
        let mut state = packed_fixture(&graph);
        for group in &groups {
            let before: Vec<_> = (0..LANES).map(|index| lane(&state, index)).collect();
            let boundary = group.boundary(&state);
            group.minimize(&graph, &mut state, &boundary);
            for (index, initial) in before.iter().enumerate() {
                let optimum = (0usize..1 << group.nodes.len())
                    .map(|bits| {
                        let mut candidate = initial.clone();
                        for (position, &node) in group.nodes.iter().enumerate() {
                            candidate[node] = if bits & (1 << position) == 0 { 1 } else { -1 };
                        }
                        (energy_f64(&candidate, &base) * 1000.0).round() as i64
                    })
                    .min()
                    .expect("enumeration");
                let actual = lane(&state, index);
                assert_eq!(
                    (energy_f64(&actual, &base) * 1000.0).round() as i64,
                    optimum
                );
                for node in 0..initial.len() {
                    if !group.nodes.contains(&node) {
                        assert_eq!(actual[node], initial[node]);
                    }
                }
            }
            assert_cache(&graph, &state);
        }
        for invalid in [vec![vec![0, 0]], vec![vec![0, 5]], vec![vec![1, 4]]] {
            assert!(Group::build(&graph, &invalid, true).is_err());
        }
        let params = SampleParams {
            num_reads: 64,
            num_sweeps: 8,
            sweeps_per_beta: 4,
            beta_range: Some((0.1, 5.0)),
            seed: 19,
        };
        let sampled = sample_clusters(&base, &params, DimerVariant::LocalBlockWeighted, &nodes)
            .expect("public overlapping-block entry point");
        let repeated = sample_clusters(&base, &params, DimerVariant::LocalBlockWeighted, &nodes)
            .expect("public overlapping-block replay");
        assert_eq!(sampled.spins.len(), 64);
        assert_eq!(sampled.spins, repeated.spins);
        assert!(sample_clusters(&base, &params, DimerVariant::BlockWeighted, &nodes).is_err());
    }

    #[test]
    fn cached_single_moves_match_direct_energy_and_keep_exact_fields() {
        let (base, _) = fixture(6, 12);
        let graph = WeightedGraph::new(&base).expect("milliweights");
        let mut state = packed_fixture(&graph);
        for threshold in [0, 30, 125, 1000, i32::MAX] {
            for node in 0..graph.neighbors.len() {
                let before: Vec<_> = (0..LANES).map(|index| lane(&state, index)).collect();
                state.single(&graph, node, threshold);
                for (index, initial) in before.iter().enumerate() {
                    let mut changed = initial.clone();
                    changed[node] *= -1;
                    let delta = ((energy_f64(&changed, &base) - energy_f64(initial, &base))
                        * 1000.0)
                        .round() as i64;
                    let expected = if delta <= 2 * i64::from(threshold) {
                        &changed
                    } else {
                        initial
                    };
                    assert_eq!(&lane(&state, index), expected);
                }
                assert_cache(&graph, &state);
            }
        }
    }

    #[test]
    fn unsupported_weights_clusters_and_frustrated_contraction_are_rejected() {
        let (mut graph, groups) = fixture(4, 1);
        let params = SampleParams {
            num_reads: 64,
            num_sweeps: 32,
            sweeps_per_beta: 4,
            beta_range: Some((0.1, 20.0)),
            seed: 8,
        };
        assert!(sample(&graph, &params, DimerVariant::ReducedWeighted, &groups).is_err());
        assert!(sample(&graph, &params, DimerVariant::PairWeighted, &groups).is_err());
        assert!(sample(&graph, &params, DimerVariant::BlockWeighted, &[vec![0, 1]]).is_err());
        graph.j[0] = 0.0001;
        assert!(sample(&graph, &params, DimerVariant::BaselineWeighted, &groups).is_err());
        graph.j[0] = SUM_LIMIT as f64;
        assert!(WeightedGraph::new(&graph).is_err());
        let (wide, groups) = fixture(9, 0);
        assert!(sample(&wide, &params, DimerVariant::BlockWeighted, &groups).is_err());
    }

    #[test]
    fn weighted_contraction_preserves_singletons_and_polish_incumbents() {
        let graph = IsingGraph::new(
            vec![0.0; 5],
            vec![-1.0, -1.0, 0.013, -0.125, 0.071, 0.089],
            vec![(0, 1), (2, 3), (0, 2), (1, 3), (0, 4), (3, 4)],
        );
        let groups = vec![vec![0, 1], vec![2, 3]];
        let params = SampleParams {
            num_reads: 65,
            num_sweeps: 32,
            sweeps_per_beta: 4,
            beta_range: Some((0.1, 50.0)),
            seed: 17,
        };
        let result = sample(
            &graph,
            &params,
            DimerVariant::ReducedPolishWeighted,
            &groups,
        )
        .expect("weighted polish");
        let repeated = sample(
            &graph,
            &params,
            DimerVariant::ReducedPolishWeighted,
            &groups,
        )
        .expect("replay");
        assert_eq!(result.spins, repeated.spins);
        let expanded = result.expanded_spins.expect("expanded states");
        let final_states = result.final_spins.expect("polish output");
        assert_eq!(expanded.len(), 65);
        let weighted = WeightedGraph::new(&graph).expect("milliweights");
        for ((selected, seed), final_state) in result.spins.iter().zip(&expanded).zip(&final_states)
        {
            assert_eq!(seed.len(), 5);
            assert_eq!(seed[0], seed[1]);
            assert_eq!(seed[2], seed[3]);
            let expected = if weighted.energy(final_state) < weighted.energy(seed) {
                final_state
            } else {
                seed
            };
            assert_eq!(selected, expected);
        }
    }

    #[test]
    fn exact_block_reaches_frustrated_six_spin_conditional_minimum() {
        let graph = IsingGraph::new(
            vec![0.0; 6],
            vec![1.0, -1.0, -1.0, -1.0, -1.0, -1.0, 1.0],
            vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 5), (0, 5), (1, 4)],
        );
        let params = SampleParams {
            num_reads: 64,
            num_sweeps: 4,
            sweeps_per_beta: 4,
            beta_range: Some((0.01, 0.01)),
            seed: 19,
        };
        let result = sample_clusters(
            &graph,
            &params,
            DimerVariant::BlockWeighted,
            &[vec![0, 1, 2, 3, 4, 5]],
        )
        .expect("frustrated six-spin blocks are supported");
        let optimum = (0..64)
            .map(|bits| {
                let spins: Vec<i8> = (0..6)
                    .map(|i| if bits & (1 << i) == 0 { 1 } else { -1 })
                    .collect();
                energy_f64(&spins, &graph)
            })
            .fold(f64::INFINITY, f64::min);
        assert_eq!(result.spins.len(), 64);
        assert!(result
            .spins
            .iter()
            .all(|s| energy_f64(s, &graph) == optimum));
    }
}
