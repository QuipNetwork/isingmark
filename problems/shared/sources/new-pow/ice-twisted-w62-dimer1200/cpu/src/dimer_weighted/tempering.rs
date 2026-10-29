//! Parallel tempering and replica disagreement moves for zero-field ice graphs.

use super::{ice, spin, Group, State, WeightedGraph, SCALE};
use crate::dimer_research::{DimerSamples, EnergyWindow, TemperingStats};
use crate::research::validate_common;
use crate::sa_msc::{MscState, LANES};
use crate::sa_sampler::read_rng;
use quip_solver_core::{IsingGraph, SampleParams};
use rand::distributions::Open01;
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use std::time::Instant;

const TEMPERATURES: usize = 32;
const MAX_TABLE_ENTRIES: usize = 2_000_000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ladder {
    Adaptive,
    Geometric,
    ProfileGeometric,
}

fn cached_energies(state: &State) -> [i64; LANES] {
    let mut energies = [0; LANES];
    for (&word, fields) in state.spins.iter().zip(&state.fields) {
        for (lane, energy) in energies.iter_mut().enumerate() {
            *energy += i64::from(spin(word, lane)) * i64::from(fields[lane]);
        }
    }
    energies.map(|value| value / 2)
}

fn swap_log_ratio(left: f64, right: f64, a: i64, b: i64) -> f64 {
    (left - right) * (a - b) as f64 / SCALE
}

fn accept_log_ratio(ratio: f64, uniform: f64) -> bool {
    ratio >= 0.0 || uniform.ln() < ratio
}

fn flip_lane(state: &mut State, lane: usize) {
    // There are no local fields: global inversion preserves every bond energy.
    for (word, fields) in state.spins.iter_mut().zip(&mut state.fields) {
        *word ^= 1 << lane;
        fields[lane] = -fields[lane];
    }
}

fn component(
    graph: &WeightedGraph,
    state: &State,
    pair: [usize; 2],
    seed: usize,
    strong: bool,
) -> Vec<usize> {
    let disagrees =
        |node: usize| spin(state.spins[node], pair[0]) != spin(state.spins[node], pair[1]);
    if !disagrees(seed) {
        return Vec::new();
    }
    let mut selected = vec![false; state.spins.len()];
    let mut nodes = vec![seed];
    selected[seed] = true;
    let mut cursor = 0;
    while cursor < nodes.len() {
        for &(other, weight) in &graph.neighbors[nodes[cursor]] {
            if weight != 0
                && (!strong || weight.abs() == 1000)
                && !selected[other]
                && disagrees(other)
            {
                selected[other] = true;
                nodes.push(other);
            }
        }
        cursor += 1;
    }
    nodes
}

fn pair_delta(graph: &WeightedGraph, state: &State, pair: [usize; 2], nodes: &[usize]) -> [i64; 2] {
    let mut selected = vec![false; state.spins.len()];
    for &node in nodes {
        selected[node] = true;
    }
    let mut delta = [0; 2];
    for &node in nodes {
        for &(other, weight) in &graph.neighbors[node] {
            if !selected[other] {
                for (change, lane) in delta.iter_mut().zip(pair) {
                    *change -= 2
                        * i64::from(weight)
                        * i64::from(spin(state.spins[node], lane))
                        * i64::from(spin(state.spins[other], lane));
                }
            }
        }
    }
    delta
}

#[derive(Default)]
struct ClusterOutcome {
    sites: usize,
    accepted: bool,
    delta: [i64; 2],
    gauge_flip: bool,
}

fn replica_move(
    graph: &WeightedGraph,
    state: &mut State,
    pair: [usize; 2],
    beta: f64,
    strong: bool,
    rng: &mut SmallRng,
) -> Result<ClusterOutcome, String> {
    let disagreement = |state: &State| {
        state
            .spins
            .iter()
            .enumerate()
            .filter_map(|(node, &word)| {
                (spin(word, pair[0]) != spin(word, pair[1])).then_some(node)
            })
            .collect::<Vec<_>>()
    };
    let mut different = disagreement(state);
    let gauge_flip = different.len() > state.spins.len() / 2;
    if gauge_flip {
        flip_lane(state, pair[0]);
        different = disagreement(state);
    }
    if different.is_empty() {
        return Ok(ClusterOutcome {
            gauge_flip,
            ..Default::default()
        });
    }
    let seed = different[rng.gen_range(0..different.len())];
    let nodes = component(graph, state, pair, seed, strong);
    let delta = pair_delta(graph, state, pair, &nodes);
    let joint_delta = delta[0] + delta[1];
    if !strong && joint_delta != 0 {
        return Err("full disagreement component changed joint energy".to_owned());
    }
    let accepted = accept_log_ratio(-beta * joint_delta as f64 / SCALE, rng.sample(Open01));
    if accepted {
        for &node in &nodes {
            state.apply(graph, node, (1 << pair[0]) | (1 << pair[1]));
        }
    }
    Ok(ClusterOutcome {
        sites: nodes.len(),
        accepted,
        delta,
        gauge_flip,
    })
}

fn adapt_ladder(
    betas: &[f64; TEMPERATURES],
    sigma: &[f64; TEMPERATURES],
    floor: f64,
) -> [f64; TEMPERATURES] {
    let mut length = [0.0; TEMPERATURES];
    for i in 1..TEMPERATURES {
        let density = ((sigma[i - 1] + sigma[i]) / 2.0).max(floor);
        length[i] = length[i - 1] + (betas[i] - betas[i - 1]) * density;
    }
    let mut result = *betas;
    let mut right = 1;
    for (index, beta) in result.iter_mut().enumerate().take(TEMPERATURES - 1).skip(1) {
        let target = length[TEMPERATURES - 1] * index as f64 / (TEMPERATURES - 1) as f64;
        while right < TEMPERATURES - 1 && length[right] < target {
            right += 1;
        }
        let fraction = (target - length[right - 1]) / (length[right] - length[right - 1]);
        *beta = betas[right - 1] + fraction * (betas[right] - betas[right - 1]);
    }
    result
}

#[derive(Clone, Copy, Default)]
struct Moments {
    count: u64,
    mean: f64,
    m2: f64,
}

impl Moments {
    fn add(&mut self, energy_milli: i64) {
        let energy = energy_milli as f64 / SCALE;
        self.count += 1;
        let delta = energy - self.mean;
        self.mean += delta / self.count as f64;
        self.m2 += delta * (energy - self.mean);
    }

    fn sigma(self) -> f64 {
        if self.count < 2 {
            0.0
        } else {
            (self.m2.max(0.0) / (self.count - 1) as f64).sqrt()
        }
    }
}

fn energy_window(moments: &[[Moments; TEMPERATURES]; 2], first_sweep: usize) -> EnergyWindow {
    EnergyWindow {
        first_sweep,
        sweeps: moments[0][0].count as usize,
        mean_energy_units: moments.map(|copy| copy.map(|item| item.mean)),
        variance_energy_units_squared: moments
            .map(|copy| copy.map(|item| item.m2.max(0.0) / item.count as f64)),
    }
}

struct Probabilities {
    quantum: i32,
    values: Vec<Vec<f64>>,
}

impl Probabilities {
    fn new(graph: &WeightedGraph, betas: &[f64; TEMPERATURES]) -> Result<Self, String> {
        let gcd = |mut a: i32, mut b: i32| {
            while b != 0 {
                (a, b) = (b, a % b);
            }
            a
        };
        let quantum = graph
            .edges
            .iter()
            .fold(0, |q, &(_, _, j)| gcd(q, j.abs()))
            .max(1);
        let maximum = graph
            .neighbors
            .iter()
            .map(|neighbors| neighbors.iter().map(|&(_, j)| j.abs()).sum::<i32>())
            .max()
            .unwrap_or(0)
            / quantum;
        let entries = maximum as usize + 1;
        if entries > MAX_TABLE_ENTRIES / TEMPERATURES {
            return Err(
                "parallel-tempering acceptance table exceeds two million entries".to_owned(),
            );
        }
        let values = betas
            .iter()
            .map(|beta| {
                (0..entries)
                    .map(|index| (-2.0 * beta * index as f64 * f64::from(quantum) / SCALE).exp())
                    .collect()
            })
            .collect();
        Ok(Self { quantum, values })
    }

    fn single_pass(
        &self,
        graph: &WeightedGraph,
        state: &mut State,
        temperatures: &[usize; LANES],
        rng: &mut SmallRng,
    ) {
        for node in 0..state.spins.len() {
            let mut mask = 0;
            for (lane, &temperature) in temperatures.iter().enumerate() {
                let half_delta = -spin(state.spins[node], lane) * state.fields[node][lane];
                if half_delta <= 0
                    || rng.gen::<f64>()
                        < self.values[temperature][(half_delta / self.quantum) as usize]
                {
                    mask |= 1 << lane;
                }
            }
            state.apply(graph, node, mask);
        }
    }
}

struct Incumbents {
    energies: [i64; LANES],
    words: Vec<u64>,
}

impl Incumbents {
    fn new(state: &State, energies: [i64; LANES]) -> Self {
        Self {
            energies,
            words: state.spins.clone(),
        }
    }

    fn observe(&mut self, state: &State, energies: &[i64; LANES]) {
        let mut mask = 0;
        for (lane, (&energy, best)) in energies.iter().zip(&mut self.energies).enumerate() {
            if energy < *best {
                *best = energy;
                mask |= 1 << lane;
            }
        }
        if mask != 0 {
            for (best, &word) in self.words.iter_mut().zip(&state.spins) {
                *best = (*best & !mask) | (word & mask);
            }
        }
    }

    fn unpack(self) -> Vec<Vec<i8>> {
        (0..LANES)
            .map(|lane| {
                self.words
                    .iter()
                    .map(|&word| spin(word, lane) as i8)
                    .collect()
            })
            .collect()
    }
}

fn record_visits(
    slots: &[[usize; TEMPERATURES]; 2],
    visits: &mut [u8; LANES],
    trips: &mut [[u64; TEMPERATURES]; 2],
) {
    for copy in slots {
        let hot = copy[0];
        let cold = copy[TEMPERATURES - 1];
        if visits[hot] == 2 {
            trips[hot / TEMPERATURES][hot % TEMPERATURES] += 1;
        }
        visits[hot] = 1;
        if visits[cold] == 1 {
            visits[cold] = 2;
        }
    }
}

fn exchange(
    slots: &mut [[usize; TEMPERATURES]; 2],
    energies: &[i64; LANES],
    betas: &[f64; TEMPERATURES],
    sweep: usize,
    rng: &mut SmallRng,
    stats: &mut TemperingStats,
) {
    for copy in slots {
        for left in (sweep % 2..TEMPERATURES - 1).step_by(2) {
            let right = left + 1;
            let ratio = swap_log_ratio(
                betas[left],
                betas[right],
                energies[copy[left]],
                energies[copy[right]],
            );
            stats.swap_attempts[left] += 1;
            if sweep >= stats.warmup_sweeps {
                stats.frozen_swap_attempts[left] += 1;
            }
            if accept_log_ratio(ratio, rng.sample(Open01)) {
                copy.swap(left, right);
                stats.swap_accepted[left] += 1;
                if sweep >= stats.warmup_sweeps {
                    stats.frozen_swap_accepted[left] += 1;
                }
            }
        }
    }
}

pub(crate) fn sample(
    graph: &IsingGraph,
    params: &SampleParams,
    groups: &[Vec<usize>],
    strong_only: bool,
    ladder: Ladder,
) -> Result<DimerSamples, String> {
    let started = Instant::now();
    validate_common(graph, params, None).map_err(|error| error.to_string())?;
    if params.num_reads != LANES
        || !params.num_sweeps.is_multiple_of(4)
        || params.sweeps_per_beta != 4
    {
        return Err("parallel tempering requires 64 reads and whole four-sweep blocks".to_owned());
    }
    let (lo, hi) = params
        .beta_range
        .ok_or("parallel tempering requires explicit beta endpoints")?;
    if lo <= 0.0 || lo >= hi {
        return Err(
            "parallel tempering requires positive, strictly increasing beta endpoints".to_owned(),
        );
    }
    let weighted = WeightedGraph::new(graph)?;
    let moves = Group::build(&weighted, groups, true)?;
    let topology = ice::IceTopology::new(&weighted, &moves)?;
    let mut ice_work = ice::IceWorkspace::new(&topology);
    let mut betas = std::array::from_fn(|i| {
        (lo.ln() + (hi.ln() - lo.ln()) * i as f64 / (TEMPERATURES - 1) as f64).exp()
    });
    betas[0] = lo;
    betas[TEMPERATURES - 1] = hi;
    let mut probabilities = Probabilities::new(&weighted, &betas)?;
    let mut rng = read_rng(params.seed, 0);
    let mut state = State::new(&weighted, MscState::random(graph.h.len(), &mut rng));
    let mut energies = cached_energies(&state);
    let mut best = Incumbents::new(&state, energies);
    let mut slots: [[usize; TEMPERATURES]; 2] = std::array::from_fn(|copy| {
        std::array::from_fn(|temperature| copy * TEMPERATURES + temperature)
    });
    let mut stats = TemperingStats {
        warmup_sweeps: (params.num_sweeps / 4).min(1024),
        initial_energies_milli: std::array::from_fn(|copy| {
            std::array::from_fn(|index| energies[copy * TEMPERATURES + index])
        }),
        ..Default::default()
    };
    let mut single_rng = SmallRng::seed_from_u64(params.seed ^ 0x5054_5349_4e47_4c45);
    let mut loop_rng = SmallRng::seed_from_u64(params.seed ^ 0x5054_4943_454c_4f4f);
    let mut swap_rng = SmallRng::seed_from_u64(params.seed ^ 0x5054_5357_4150_3031);
    let mut cluster_rng = SmallRng::seed_from_u64(params.seed ^ 0x5054_434c_5553_5445);
    let mut moments = [Moments::default(); TEMPERATURES];
    let mut energy_moments = [[Moments::default(); TEMPERATURES]; 2];
    let mut window_start = 0;
    let mut visits = [0; LANES];
    let mut frozen_visits = [0; LANES];
    record_visits(&slots, &mut visits, &mut stats.round_trips);
    let density_floor = 0.05
        * weighted
            .edges
            .iter()
            .map(|&(_, _, j)| j.abs())
            .max()
            .unwrap_or(1) as f64
        / SCALE;
    for sweep in 0..params.num_sweeps {
        let mut temperatures = [0; LANES];
        for copy in &slots {
            for (temperature, &lane) in copy.iter().enumerate() {
                temperatures[lane] = temperature;
            }
        }
        probabilities.single_pass(&weighted, &mut state, &temperatures, &mut single_rng);
        for (lane, &temperature) in temperatures.iter().enumerate() {
            for _ in 0..4 {
                ice_work.step(
                    &topology,
                    &weighted,
                    &mut state,
                    lane,
                    betas[temperature],
                    &mut loop_rng,
                );
            }
        }
        energies = cached_energies(&state);
        best.observe(&state, &energies);
        if (sweep + 1).is_multiple_of(4) {
            for (temperature, &beta) in betas.iter().enumerate().skip(TEMPERATURES / 2) {
                let pair = [slots[0][temperature], slots[1][temperature]];
                let outcome = replica_move(
                    &weighted,
                    &mut state,
                    pair,
                    beta,
                    strong_only,
                    &mut cluster_rng,
                )?;
                stats.cluster_attempts += 1;
                stats.gauge_flips += u64::from(outcome.gauge_flip);
                if outcome.sites > 0 {
                    stats.cluster_formed += 1;
                    stats.cluster_proposed_sites += outcome.sites as u64;
                    stats.cluster_max_sites = stats.cluster_max_sites.max(outcome.sites);
                    stats.nonzero_joint_delta += u64::from(outcome.delta.iter().sum::<i64>() != 0);
                }
                if outcome.accepted {
                    stats.cluster_accepted += 1;
                    stats.cluster_accepted_sites += outcome.sites as u64;
                    for (lane, change) in pair.into_iter().zip(outcome.delta) {
                        energies[lane] += change;
                    }
                }
            }
            best.observe(&state, &energies);
        }
        if ladder == Ladder::ProfileGeometric {
            for (copy, lanes) in slots.iter().enumerate() {
                for (temperature, &lane) in lanes.iter().enumerate() {
                    energy_moments[copy][temperature].add(energies[lane]);
                }
            }
            let completed = sweep + 1;
            if completed.is_multiple_of(256) || completed == params.num_sweeps {
                stats
                    .energy_windows
                    .push(energy_window(&energy_moments, window_start));
                energy_moments = [[Moments::default(); TEMPERATURES]; 2];
                window_start = completed;
            }
        }
        if ladder == Ladder::Adaptive && sweep < stats.warmup_sweeps {
            for copy in &slots {
                for (temperature, &lane) in copy.iter().enumerate() {
                    moments[temperature].add(energies[lane]);
                }
            }
        }
        exchange(
            &mut slots,
            &energies,
            &betas,
            sweep,
            &mut swap_rng,
            &mut stats,
        );
        record_visits(&slots, &mut visits, &mut stats.round_trips);
        if sweep >= stats.warmup_sweeps {
            record_visits(&slots, &mut frozen_visits, &mut stats.frozen_round_trips);
        }
        let completed = sweep + 1;
        if ladder == Ladder::Adaptive
            && completed <= stats.warmup_sweeps
            && (completed.is_multiple_of(256) || completed == stats.warmup_sweeps)
        {
            betas = adapt_ladder(&betas, &moments.map(Moments::sigma), density_floor);
            if betas.iter().any(|beta| !beta.is_finite())
                || betas.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err("adapted temperature ladder lost finite strict ordering".to_owned());
            }
            probabilities = Probabilities::new(&weighted, &betas)?;
            moments = [Moments::default(); TEMPERATURES];
            stats.ladder_updates += 1;
        }
        if completed == stats.warmup_sweeps {
            record_visits(&slots, &mut frozen_visits, &mut stats.frozen_round_trips);
        }
    }
    stats.final_betas = betas;
    stats.final_energies_milli = std::array::from_fn(|copy| {
        std::array::from_fn(|index| energies[copy * TEMPERATURES + index])
    });
    ice_work.stats.tempering = Some(stats);
    Ok(DimerSamples {
        spins: best.unpack(),
        expanded_spins: None,
        final_spins: None,
        solve_seconds: started.elapsed().as_secs_f64(),
        polish_seconds: 0.0,
        solve_sweeps: params.num_sweeps,
        polish_sweeps: 0,
        ice_loop_stats: Some(ice_work.stats),
    })
}

pub(crate) fn sample_band(
    graph: &IsingGraph,
    params: &SampleParams,
    groups: &[Vec<usize>],
    strong_only: bool,
) -> Result<DimerSamples, String> {
    let started = Instant::now();
    validate_common(graph, params, None).map_err(|error| error.to_string())?;
    if params.num_sweeps == 0 || !params.num_sweeps.is_multiple_of(32) {
        return Err(
            "band tempering requires whole four-sweep blocks in both 7:1 stages".to_owned(),
        );
    }
    let (_, cold) = params
        .beta_range
        .ok_or("band tempering requires explicit beta endpoints")?;
    let peak = graph.j.iter().map(|j| j.abs()).fold(0.0, f64::max);
    if peak <= 0.0 || !peak.is_finite() {
        return Err("band tempering requires a finite nonzero coupling scale".to_owned());
    }
    let band = (0.4 / peak, 3.0 / peak);
    if cold <= band.1 {
        return Err("the job cold endpoint must exceed the band cold endpoint".to_owned());
    }
    let polish_sweeps = params.num_sweeps / 8;
    let solve_params = SampleParams {
        num_sweeps: params.num_sweeps - polish_sweeps,
        beta_range: Some(band),
        ..params.clone()
    };
    let mut output = sample(graph, &solve_params, groups, strong_only, Ladder::Geometric)?;
    output.solve_seconds = started.elapsed().as_secs_f64();

    let polish_started = Instant::now();
    let weighted = WeightedGraph::new(graph)?;
    let moves = Group::build(&weighted, groups, true)?;
    let topology = ice::IceTopology::new(&weighted, &moves)?;
    let polish_params = SampleParams {
        num_sweeps: polish_sweeps,
        beta_range: Some((band.1, cold)),
        seed: params.seed ^ 0x4241_4e44_434f_4c44,
        ..params.clone()
    };
    let (final_states, polish_stats) = super::anneal(
        graph,
        &polish_params,
        &moves,
        true,
        Some(&output.spins),
        Some((&topology, 64, false)),
        false,
    )?;
    output.expanded_spins = Some(output.spins.clone());
    for (incumbent, candidate) in output.spins.iter_mut().zip(&final_states) {
        if weighted.energy(candidate) < weighted.energy(incumbent) {
            incumbent.clone_from(candidate);
        }
    }
    let stats = output
        .ice_loop_stats
        .as_mut()
        .ok_or("tempering returned no move counts")?;
    stats.attempts += polish_stats.attempts;
    stats.formed_loops += polish_stats.formed_loops;
    stats.formed_strings += polish_stats.formed_strings;
    stats.accepted += polish_stats.accepted;
    stats.flipped_spins += polish_stats.flipped_spins;
    output.final_spins = Some(final_states);
    output.polish_sweeps = polish_sweeps;
    output.polish_seconds = polish_started.elapsed().as_secs_f64();
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::super::tests::{assert_cache, lane};
    use super::*;

    fn fixture() -> (IsingGraph, Vec<Vec<usize>>) {
        let mut edges = Vec::new();
        for u in 0..4 {
            for v in u + 1..4 {
                edges.push((u, v));
            }
        }
        edges.extend([(0, 4), (1, 4)]);
        let mut j = vec![1.0; 6];
        j.extend([0.125, -0.25]);
        (
            IsingGraph::new(vec![0.0; 5], j, edges),
            vec![vec![0, 1, 2, 3]],
        )
    }

    fn state(graph: &WeightedGraph, a: &[i8], b: &[i8]) -> State {
        let mut rng = SmallRng::seed_from_u64(19);
        let seeds: Vec<_> = (0..LANES).map(|i| if i % 2 == 0 { a } else { b }).collect();
        State::new(graph, MscState::seeded(a.len(), &seeds, &mut rng))
    }

    fn params(sweeps: usize) -> SampleParams {
        SampleParams {
            num_reads: 64,
            num_sweeps: sweeps,
            sweeps_per_beta: 4,
            beta_range: Some((0.05, 20.0)),
            seed: 197,
        }
    }

    #[test]
    fn cached_energy_and_global_gauge_match_direct_edges() {
        let (base, _) = fixture();
        let g = WeightedGraph::new(&base).unwrap();
        let mut s = state(&g, &[1, -1, 1, -1, 1], &[-1, 1, 1, 1, 1]);
        let before = cached_energies(&s);
        for (i, e) in before.iter().enumerate() {
            assert_eq!(*e, g.energy(&lane(&s, i)));
        }
        let original = lane(&s, 0);
        flip_lane(&mut s, 0);
        assert_eq!(lane(&s, 0), original.iter().map(|v| -v).collect::<Vec<_>>());
        assert_eq!(cached_energies(&s), before);
        assert_cache(&g, &s);
    }

    #[test]
    fn exchange_uses_joint_boltzmann_ratio_and_log_uniform() {
        // beta 1 and 2, energies -3 and -1 energy units: swapping gains exp(2).
        assert_eq!(swap_log_ratio(1.0, 2.0, -3000, -1000), 2.0);
        assert_eq!(swap_log_ratio(1.0, 2.0, -1000, -3000), -2.0);
        assert!(accept_log_ratio(2.0, 0.99));
        assert!(accept_log_ratio(-2.0, 0.1));
        assert!(!accept_log_ratio(-2.0, 0.2));
    }

    #[test]
    fn full_disagreement_component_preserves_joint_energy_and_caches() {
        let (base, _) = fixture();
        let g = WeightedGraph::new(&base).unwrap();
        for bits_a in 0..32 {
            for bits_b in 0..32 {
                let a: Vec<_> = (0..5)
                    .map(|i| if bits_a & (1 << i) == 0 { 1 } else { -1 })
                    .collect();
                let b: Vec<_> = (0..5)
                    .map(|i| if bits_b & (1 << i) == 0 { 1 } else { -1 })
                    .collect();
                let mut s = state(&g, &a, &b);
                for seed in 0..5 {
                    if a[seed] == b[seed] {
                        continue;
                    }
                    let nodes = component(&g, &s, [0, 1], seed, false);
                    assert!(nodes.contains(&seed));
                    let delta = pair_delta(&g, &s, [0, 1], &nodes);
                    assert_eq!(delta[0] + delta[1], 0);
                    let before = [g.energy(&lane(&s, 0)), g.energy(&lane(&s, 1))];
                    for &node in &nodes {
                        s.apply(&g, node, 3);
                    }
                    for i in 0..2 {
                        assert_eq!(g.energy(&lane(&s, i)) - before[i], delta[i]);
                    }
                    assert_cache(&g, &s);
                    for &node in &nodes {
                        s.apply(&g, node, 3);
                    }
                }
            }
        }
    }

    #[test]
    fn strong_components_charge_omitted_weak_edges_exactly() {
        let base = IsingGraph::new(vec![0.0; 3], vec![1.0, 0.125], vec![(0, 1), (0, 2)]);
        let g = WeightedGraph::new(&base).unwrap();
        let mut s = state(&g, &[1, 1, 1], &[-1, -1, -1]);
        let nodes = component(&g, &s, [0, 1], 0, true);
        assert_eq!(nodes, vec![0, 1]);
        let delta = pair_delta(&g, &s, [0, 1], &nodes);
        assert_eq!(delta, [-250, -250]);
        for &node in &nodes {
            s.apply(&g, node, 3);
        }
        assert_eq!(pair_delta(&g, &s, [0, 1], &nodes), [250, 250]);
        assert!(!accept_log_ratio(-10.0 * 500.0 / SCALE, 0.5));
        assert_cache(&g, &s);
    }

    #[test]
    fn adaptation_preserves_endpoints_and_concentrates_temperatures() {
        let betas = std::array::from_fn(|i| 1.0 + i as f64);
        let sigma = std::array::from_fn(|i| if i < 8 { 100.0 } else { 0.0 });
        let next = adapt_ladder(&betas, &sigma, 0.05);
        assert_eq!(next[0], 1.0);
        assert_eq!(next[31], 32.0);
        assert!(next.windows(2).all(|w| w[0] < w[1]));
        assert!(next.iter().filter(|b| **b < 9.0).count() > 25);
        let flat = adapt_ladder(&betas, &[0.0; 32], 0.05);
        for (a, b) in flat.iter().zip(betas) {
            assert!((*a - b).abs() < 1e-12);
        }
    }

    #[test]
    fn sampler_replays_and_retains_best_within_the_requested_budget() {
        let (graph, groups) = fixture();
        let g = WeightedGraph::new(&graph).unwrap();
        for strong in [false, true] {
            let a = sample(&graph, &params(32), &groups, strong, Ladder::Adaptive).unwrap();
            let b = sample(&graph, &params(32), &groups, strong, Ladder::Adaptive).unwrap();
            assert_eq!(a.spins, b.spins);
            assert_eq!(a.spins.len(), 64);
            assert_eq!(a.solve_sweeps, 32);
            assert_eq!(a.polish_sweeps, 0);
            let stats = a.ice_loop_stats.unwrap();
            let pt = stats.tempering.as_ref().unwrap();
            assert_eq!(stats.attempts, 32 * 64 * 4);
            assert_eq!(pt.warmup_sweeps, 8);
            assert_eq!(pt.ladder_updates, 1);
            assert_eq!(pt.cluster_attempts, 32 / 4 * 16);
            assert_eq!(pt.swap_attempts.iter().sum::<u64>(), 32 * 31);
            assert_eq!(pt.frozen_swap_attempts.iter().sum::<u64>(), 24 * 31);
            assert_eq!(
                serde_json::to_value(&stats).unwrap(),
                serde_json::to_value(b.ice_loop_stats.unwrap()).unwrap()
            );
            for (i, spins) in a.spins.iter().enumerate() {
                let e = g.energy(spins);
                assert!(e <= pt.initial_energies_milli[i / 32][i % 32]);
                assert!(e <= pt.final_energies_milli[i / 32][i % 32]);
            }
        }
    }

    #[test]
    fn independent_metropolis_draws_separate_identical_lanes() {
        let g =
            WeightedGraph::new(&IsingGraph::new(vec![0.0; 2], vec![1.0], vec![(0, 1)])).unwrap();
        let mut s = state(&g, &[1, -1], &[1, -1]);
        let probabilities = Probabilities::new(&g, &[2.0_f64.ln() / 2.0; 32]).unwrap();
        probabilities.single_pass(&g, &mut s, &[0; 64], &mut SmallRng::seed_from_u64(7));
        assert!(s.spins[0].count_ones() > 0 && s.spins[0].count_ones() < 64);
        assert_cache(&g, &s);
    }

    #[test]
    fn exchanges_permute_assignments_and_round_trips_require_both_endpoints() {
        let mut slots: [[usize; 32]; 2] =
            std::array::from_fn(|c| std::array::from_fn(|i| 32 * c + i));
        let mut visits = [0; 64];
        let mut trips = [[0; 32]; 2];
        record_visits(&slots, &mut visits, &mut trips);
        record_visits(&slots, &mut visits, &mut trips);
        assert_eq!(trips, [[0; 32]; 2]);
        slots[0].swap(0, 31);
        record_visits(&slots, &mut visits, &mut trips);
        assert_eq!(trips, [[0; 32]; 2]);
        slots[0].swap(0, 31);
        record_visits(&slots, &mut visits, &mut trips);
        assert_eq!(trips[0][0], 1);
        let original = slots;
        let mut stats = TemperingStats::default();
        exchange(
            &mut slots,
            &[0; 64],
            &[1.0; 32],
            0,
            &mut SmallRng::seed_from_u64(5),
            &mut stats,
        );
        for copy in 0..2 {
            for i in (0..32).step_by(2) {
                assert_eq!(slots[copy][i], original[copy][i + 1]);
                assert_eq!(slots[copy][i + 1], original[copy][i]);
            }
        }
        assert_eq!(stats.swap_accepted.iter().sum::<u64>(), 32);
    }

    #[test]
    fn incumbents_ignore_higher_and_equal_energy_states() {
        let g =
            WeightedGraph::new(&IsingGraph::new(vec![0.0; 2], vec![1.0], vec![(0, 1)])).unwrap();
        let mut s = state(&g, &[1, 1], &[1, 1]);
        let mut best = Incumbents::new(&s, cached_energies(&s));
        s.apply(&g, 0, u64::MAX);
        best.observe(&s, &cached_energies(&s));
        let saved = best.words.clone();
        flip_lane(&mut s, 0);
        best.observe(&s, &cached_energies(&s));
        assert_eq!(best.words, saved);
        s.apply(&g, 1, u64::MAX);
        best.observe(&s, &cached_energies(&s));
        assert_eq!(best.words, saved);
        assert!(best.unpack().iter().all(|spins| g.energy(spins) == -1000));
    }

    #[test]
    fn moments_use_energy_units_and_tables_refuse_excessive_allocation() {
        let mut moments = Moments::default();
        moments.add(1000);
        moments.add(3000);
        assert_eq!(moments.mean, 2.0);
        assert!((moments.sigma() - 2.0_f64.sqrt()).abs() < 1e-12);
        let g = WeightedGraph::new(&IsingGraph::new(
            vec![0.0; 3],
            vec![100.0, 0.001],
            vec![(0, 1), (0, 2)],
        ))
        .unwrap();
        assert!(Probabilities::new(&g, &[1.0; 32]).is_err());
    }

    #[test]
    fn replica_engine_accepts_and_rejects_using_full_energy() {
        let graph = IsingGraph::new(vec![0.0; 7], vec![1.0, 0.125], vec![(0, 1), (0, 2)]);
        let g = WeightedGraph::new(&graph).unwrap();
        let mut s = state(&g, &[1, 1, -1, 1, 1, 1, 1], &[-1, -1, 1, 1, 1, 1, 1]);
        let original = s.spins.clone();
        let rejected = replica_move(
            &g,
            &mut s,
            [0, 1],
            1e6,
            true,
            &mut SmallRng::seed_from_u64(13),
        )
        .unwrap();
        assert!(!rejected.accepted && !rejected.gauge_flip && rejected.sites > 0);
        assert_eq!(rejected.delta, [250, 250]);
        assert_eq!(s.spins, original);
        assert_cache(&g, &s);
        let before = cached_energies(&s);
        let accepted = replica_move(
            &g,
            &mut s,
            [0, 1],
            0.0,
            true,
            &mut SmallRng::seed_from_u64(13),
        )
        .unwrap();
        assert!(accepted.accepted);
        let after = cached_energies(&s);
        for i in 0..2 {
            assert_eq!(after[i] - before[i], accepted.delta[i]);
        }
        assert_cache(&g, &s);
        flip_lane(&mut s, 0);
        let gauged = replica_move(
            &g,
            &mut s,
            [0, 1],
            0.0,
            false,
            &mut SmallRng::seed_from_u64(13),
        )
        .unwrap();
        assert!(gauged.gauge_flip && gauged.accepted);
        assert_eq!(gauged.delta.iter().sum::<i64>(), 0);
        assert_cache(&g, &s);
    }

    #[test]
    fn adaptation_boundaries_keep_exact_frozen_exchange_counts() {
        let (graph, groups) = fixture();
        for sweeps in [4, 1028, 8192] {
            let result = sample(&graph, &params(sweeps), &groups, false, Ladder::Adaptive).unwrap();
            let stats = result.ice_loop_stats.unwrap().tempering.unwrap();
            let warmup = (sweeps / 4).min(1024);
            assert_eq!(stats.warmup_sweeps, warmup);
            assert_eq!(stats.ladder_updates, warmup.div_ceil(256) as u64);
            assert_eq!(stats.final_betas[0], 0.05);
            assert_eq!(stats.final_betas[31], 20.0);
            assert!(stats.final_betas.windows(2).all(|w| w[0] < w[1]));
            for edge in 0..31 {
                let expected = 2 * (warmup..sweeps).filter(|step| step % 2 == edge % 2).count();
                assert_eq!(stats.frozen_swap_attempts[edge], expected as u64);
                assert!(stats.frozen_swap_accepted[edge] <= expected as u64);
            }
        }
    }

    #[test]
    fn acceptance_table_covers_mixed_signed_maximum_fields() {
        let graph = IsingGraph::new(vec![0.0; 3], vec![0.125, -0.25], vec![(0, 1), (0, 2)]);
        let g = WeightedGraph::new(&graph).unwrap();
        let p = Probabilities::new(&g, &[2.0; 32]).unwrap();
        assert_eq!(p.quantum, 125);
        assert_eq!(p.values[0].len(), 4);
        assert!((p.values[0][3] - (-1.5_f64).exp()).abs() < 1e-15);
        let mut s = state(&g, &[1, -1, 1], &[1, -1, 1]);
        assert_eq!(-spin(s.spins[0], 0) * s.fields[0][0], 375);
        p.single_pass(&g, &mut s, &[31; 64], &mut SmallRng::seed_from_u64(19));
        assert_cache(&g, &s);
    }

    #[test]
    fn sampler_rejects_unsupported_budgets_fields_and_temperatures() {
        let (mut graph, groups) = fixture();
        let mut p = params(32);
        p.num_reads = 63;
        assert!(sample(&graph, &p, &groups, false, Ladder::Adaptive).is_err());
        p = params(30);
        assert!(sample(&graph, &p, &groups, false, Ladder::Adaptive).is_err());
        for range in [
            None,
            Some((0.0, 20.0)),
            Some((1.0, 1.0)),
            Some((2.0, 1.0)),
            Some((1.0, f64::INFINITY)),
        ] {
            p = params(32);
            p.beta_range = range;
            assert!(sample(&graph, &p, &groups, false, Ladder::Adaptive).is_err());
        }
        graph.h[0] = 0.125;
        assert!(sample(&graph, &params(32), &groups, false, Ladder::Adaptive).is_err());
        assert!(sample(&graph, &params(32), &groups, true, Ladder::Adaptive).is_err());
    }
    #[test]
    fn geometric_ladder_stays_fixed_with_the_original_exchange_budget() {
        let (graph, groups) = fixture();
        for sweeps in [4, 1028, 8192] {
            for strong in [false, true] {
                let p = params(sweeps);
                let result = sample(&graph, &p, &groups, strong, Ladder::Geometric).unwrap();
                let stats = result.ice_loop_stats.unwrap().tempering.unwrap();
                assert_eq!(stats.ladder_updates, 0);
                assert_eq!(stats.warmup_sweeps, (sweeps / 4).min(1024));
                let (lo, hi) = p.beta_range.unwrap();
                assert_eq!(stats.final_betas[0], lo);
                assert_eq!(stats.final_betas[31], hi);
                for (index, beta) in stats.final_betas.iter().enumerate() {
                    let expected = lo * (hi / lo).powf(index as f64 / 31.0);
                    assert!((beta - expected).abs() < 1e-12 * expected);
                }
                for edge in 0..31 {
                    let count = |range: std::ops::Range<usize>| {
                        2 * range.filter(|sweep| sweep % 2 == edge % 2).count() as u64
                    };
                    assert_eq!(stats.swap_attempts[edge], count(0..sweeps));
                    assert_eq!(
                        stats.frozen_swap_attempts[edge],
                        count(stats.warmup_sweeps..sweeps)
                    );
                }
            }
        }
    }

    #[test]
    fn geometric_retained_states_match_exact_energy_checkpoints() {
        let (graph, groups) = fixture();
        let weighted = WeightedGraph::new(&graph).unwrap();
        for strong in [false, true] {
            let result = sample(&graph, &params(1028), &groups, strong, Ladder::Geometric).unwrap();
            let stats = result.ice_loop_stats.unwrap().tempering.unwrap();
            assert_eq!(result.spins.len(), 64);
            for (lane, spins) in result.spins.iter().enumerate() {
                let energy = weighted.energy(spins);
                assert!(energy <= stats.initial_energies_milli[lane / 32][lane % 32]);
                assert!(energy <= stats.final_energies_milli[lane / 32][lane % 32]);
            }
        }
    }

    #[test]
    fn profile_windows_separate_copy_offsets_from_temporal_variance() {
        let mut moments = [[Moments::default(); TEMPERATURES]; 2];
        for (copy, center) in moments.iter_mut().zip([0, 10000]) {
            for moment in copy {
                moment.add(center - 1000);
                moment.add(center + 1000);
            }
        }
        let window = energy_window(&moments, 256);
        assert_eq!(window.first_sweep, 256);
        assert_eq!(window.sweeps, 2);
        assert_eq!(window.mean_energy_units[0], [0.0; 32]);
        assert_eq!(window.mean_energy_units[1], [10.0; 32]);
        assert_eq!(window.variance_energy_units_squared, [[1.0; 32]; 2]);
    }

    #[test]
    fn profile_public_routes_preserve_sampling_and_flush_partial_windows() {
        use crate::dimer_research::{sample_clusters, DimerVariant};
        let (graph, groups) = fixture();
        for (base, profile) in [
            (
                DimerVariant::PtIceGeometricFullWeighted,
                DimerVariant::PtIceProfileFullWeighted,
            ),
            (
                DimerVariant::PtIceGeometricStrongWeighted,
                DimerVariant::PtIceProfileStrongWeighted,
            ),
        ] {
            for sweeps in [4, 260, 1028] {
                let p = params(sweeps);
                let expected = sample_clusters(&graph, &p, base, &groups).unwrap();
                let actual = sample_clusters(&graph, &p, profile, &groups).unwrap();
                assert_eq!(actual.spins, expected.spins);
                let mut stats = actual.ice_loop_stats.unwrap();
                let windows = std::mem::take(&mut stats.tempering.as_mut().unwrap().energy_windows);
                assert_eq!(windows.len(), sweeps.div_ceil(256));
                assert_eq!(
                    windows.iter().map(|window| window.sweeps).sum::<usize>(),
                    sweeps
                );
                assert_eq!(windows[0].first_sweep, 0);
                assert_eq!(windows.last().unwrap().sweeps, 4);
                assert_eq!(
                    serde_json::to_value(&stats).unwrap(),
                    serde_json::to_value(expected.ice_loop_stats.unwrap()).unwrap()
                );
            }
        }
    }
    #[test]
    fn band_stages_share_budget_and_retain_the_better_physical_state() {
        use crate::dimer_research::{sample_clusters, DimerVariant};
        let (graph, groups) = fixture();
        let weighted = WeightedGraph::new(&graph).unwrap();
        for variant in [
            DimerVariant::PtIceBandFullWeighted,
            DimerVariant::PtIceBandStrongWeighted,
        ] {
            for (total, main, descent, attempts) in [
                (32, 28, 4, 23552),
                (96, 84, 12, 70656),
                (1024, 896, 128, 753664),
            ] {
                let output = sample_clusters(&graph, &params(total), variant, &groups).unwrap();
                assert_eq!((output.solve_sweeps, output.polish_sweeps), (main, descent));
                let before = output.expanded_spins.as_ref().expect("descent seeds");
                let after = output.final_spins.as_ref().expect("raw descent output");
                assert_eq!(
                    (before.len(), after.len(), output.spins.len()),
                    (64, 64, 64)
                );
                for ((seed, candidate), retained) in before.iter().zip(after).zip(&output.spins) {
                    assert_eq!(
                        weighted.energy(retained),
                        weighted.energy(seed).min(weighted.energy(candidate))
                    );
                }
                let loops = output.ice_loop_stats.unwrap();
                assert_eq!(loops.attempts, attempts);
                let pt = loops.tempering.unwrap();
                assert_eq!((pt.final_betas[0], pt.final_betas[31]), (0.4, 3.0));
                assert_eq!(pt.cluster_attempts, (main / 4 * 16) as u64);
                assert_eq!(pt.ladder_updates, 0);
                assert!(output.solve_seconds > 0.0 && output.polish_seconds > 0.0);
            }
        }
    }

    #[test]
    fn band_wrapper_preserves_the_fixed_geometric_prefix() {
        use crate::dimer_research::{sample_clusters, DimerVariant};
        let (graph, groups) = fixture();
        for (variant, strong) in [
            (DimerVariant::PtIceBandFullWeighted, false),
            (DimerVariant::PtIceBandStrongWeighted, true),
        ] {
            let band = sample_clusters(&graph, &params(96), variant, &groups).unwrap();
            let main_params = SampleParams {
                num_sweeps: 84,
                beta_range: Some((0.4, 3.0)),
                ..params(96)
            };
            let prefix = sample(&graph, &main_params, &groups, strong, Ladder::Geometric).unwrap();
            assert_eq!(band.expanded_spins.as_ref().unwrap(), &prefix.spins);
            assert_eq!(
                serde_json::to_value(band.ice_loop_stats.unwrap().tempering).unwrap(),
                serde_json::to_value(prefix.ice_loop_stats.unwrap().tempering).unwrap()
            );
        }
    }

    #[test]
    fn band_rejects_partial_descent_blocks_and_warm_cold_endpoint() {
        let (graph, groups) = fixture();
        for sweeps in [0, 4, 12, 36] {
            assert!(sample_band(&graph, &params(sweeps), &groups, false).is_err());
        }
        let warm = SampleParams {
            beta_range: Some((0.05, 2.0)),
            ..params(32)
        };
        assert!(sample_band(&graph, &warm, &groups, false).is_err());
    }
}
