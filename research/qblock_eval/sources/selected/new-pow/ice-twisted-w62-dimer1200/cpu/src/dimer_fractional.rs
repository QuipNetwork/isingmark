//! Packed simulated annealing for the fractional dimer instances.
//!
//! This kernel keeps the source `f64` coefficient (`0.031` or `0.062`). It
//! never expands a fractional edge into unit edges. One word advances 64
//! replicas, and each random Metropolis threshold is shared by those lanes.

use quip_solver_core::{IsingGraph, SampleParams};
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

use crate::coloring::Coloring;
use crate::sa_int::draw_row;
use crate::sa_msc::{MscState, LANES};
use crate::sa_sampler::read_rng;
use crate::sampler_core::{build_beta_schedule, effective_sweeps_per_beta, CpuGraph};

const SINGLE_MAX_EXTERNAL: usize = 20;
const PAIR_MAX_EXTERNAL: usize = 38;
const PLANES: usize = 6;
const SINGLE_SALT: u64 = 0x5341_5F54_424C_4531;
const PAIR_SALT: u64 = 0x4449_4D45_525F_3031;

#[derive(Clone, Copy)]
struct ExternalEdge {
    node: usize,
    negative: bool,
}

struct FractionalGraph {
    external: Vec<Vec<ExternalEdge>>,
    partner: Vec<Option<usize>>,
    pairs: Vec<(usize, usize)>,
    other: f64,
}

impl FractionalGraph {
    fn build(graph: &IsingGraph) -> Result<Self, String> {
        let n = graph.h.len();
        if graph.h.iter().any(|&h| h != 0.0) {
            return Err("the fractional dimer kernel requires zero fields".to_owned());
        }
        if graph.edges.len() != graph.j.len() {
            return Err("the edge and coupling arrays must have equal length".to_owned());
        }

        let mut external = vec![Vec::new(); n];
        let mut partner = vec![None; n];
        let mut pairs = Vec::new();
        let mut other = None;
        for (&(u, v), &j) in graph.edges.iter().zip(&graph.j) {
            if u >= n || v >= n || u == v {
                return Err(format!("invalid edge ({u}, {v}) for {n} variables"));
            }
            if j == -1.0 {
                if partner[u].is_some() || partner[v].is_some() {
                    return Err("the -1.0 bonds must form a matching".to_owned());
                }
                partner[u] = Some(v);
                partner[v] = Some(u);
                pairs.push((u.min(v), u.max(v)));
                continue;
            }
            if !j.is_finite() || (j.abs() != 0.031 && j.abs() != 0.062) {
                return Err(format!("unsupported external coupling {j}"));
            }
            match other {
                Some(value) if value != j.abs() => {
                    return Err("all external bonds must have one magnitude".to_owned());
                }
                None => other = Some(j.abs()),
                Some(_) => {}
            }
            let edge = ExternalEdge {
                node: v,
                negative: j < 0.0,
            };
            external[u].push(edge);
            external[v].push(ExternalEdge {
                node: u,
                negative: edge.negative,
            });
        }
        if external.iter().any(|row| row.len() > SINGLE_MAX_EXTERNAL) {
            return Err(format!("external degree exceeds {SINGLE_MAX_EXTERNAL}"));
        }
        pairs.sort_unstable();
        for &(u, v) in &pairs {
            if external[u].len() + external[v].len() > PAIR_MAX_EXTERNAL {
                return Err(format!(
                    "dimer ({u}, {v}) has more than {PAIR_MAX_EXTERNAL} external bonds"
                ));
            }
        }
        let other = other.ok_or("the graph has no fractional external bond")?;
        Ok(Self {
            external,
            partner,
            pairs,
            other,
        })
    }
}

#[derive(Clone, Copy, Default)]
struct Threshold {
    aligned: i16,
    broken: i16,
    external: i16,
}

/// Largest integer `m` for which `other * m + offset <= t`.
///
/// The quotient supplies the fast candidate. The short correction uses the
/// original source coefficient and the same floating expression as the direct
/// energy test. This repairs division/floor rounding exactly at an integer
/// boundary instead of treating `0.031` or `0.062` as a dyadic surrogate.
fn signed_threshold(t: f64, offset: f64, other: f64) -> i16 {
    const LIMIT: i32 = PAIR_MAX_EXTERNAL as i32 + 2;
    if t.is_infinite() {
        return LIMIT as i16;
    }
    let quotient = ((t - offset) / other).floor();
    let mut m = if quotient <= -f64::from(LIMIT) {
        -LIMIT
    } else if quotient >= f64::from(LIMIT) {
        LIMIT
    } else {
        quotient as i32
    };
    while m > -LIMIT && other * f64::from(m) + offset > t {
        m -= 1;
    }
    while m < LIMIT && other * f64::from(m + 1) + offset <= t {
        m += 1;
    }
    m as i16
}

fn fill_threshold_row(beta: f64, other: f64, rng: &mut SmallRng, row: &mut [Threshold]) {
    for slot in row {
        let u = rng.gen::<f64>();
        // Standard's f64 draw is in [0, 1). The zero endpoint has the correct
        // limiting threshold: every finite energy change is accepted.
        let t = if u == 0.0 {
            f64::INFINITY
        } else {
            -u.ln() / (2.0 * beta)
        };
        *slot = Threshold {
            aligned: signed_threshold(t, 1.0, other),
            broken: signed_threshold(t, -1.0, other),
            external: signed_threshold(t, 0.0, other),
        };
    }
}

#[inline]
fn add_plane(planes: &mut [u64; PLANES], input: u64) {
    let mut carry = input;
    for plane in planes {
        let next = *plane & carry;
        *plane ^= carry;
        carry = next;
    }
}

fn count_external(edges: &[ExternalEdge], owner: usize, spins: &[u64]) -> [u64; PLANES] {
    let mut planes = [0; PLANES];
    let own = spins[owner];
    for edge in edges {
        let sign = if edge.negative { u64::MAX } else { 0 };
        add_plane(&mut planes, sign ^ own ^ spins[edge.node]);
    }
    planes
}

fn count_pair_external(
    graph: &FractionalGraph,
    u: usize,
    v: usize,
    spins: &[u64],
) -> [u64; PLANES] {
    let mut planes = [0; PLANES];
    for owner in [u, v] {
        let own = spins[owner];
        for edge in &graph.external[owner] {
            let sign = if edge.negative { u64::MAX } else { 0 };
            add_plane(&mut planes, sign ^ own ^ spins[edge.node]);
        }
    }
    planes
}

#[inline]
fn le_constant(planes: &[u64; PLANES], limit: usize) -> u64 {
    let bound = limit + 1;
    if bound >= 1 << PLANES {
        return u64::MAX;
    }
    let mut ge = u64::MAX;
    for (bit, &plane) in planes.iter().enumerate() {
        if bound & (1 << bit) == 0 {
            ge |= plane;
        } else {
            ge &= plane;
        }
    }
    !ge
}

#[inline]
fn mask_for_threshold(planes: &[u64; PLANES], degree: usize, m: i16) -> u64 {
    let m = i32::from(m);
    let degree = degree as i32;
    if m >= degree {
        return u64::MAX;
    }
    if degree + m < 0 {
        return 0;
    }
    le_constant(planes, ((degree + m) / 2) as usize)
}

fn sweep_word(
    graph: &FractionalGraph,
    row: &[Threshold],
    nodes: &[u32],
    offset: usize,
    state: &mut MscState,
) {
    let row_mask = row.len() - 1;
    for &node in nodes {
        let node = node as usize;
        let spins = state.spins();
        let planes = count_external(&graph.external[node], node, spins);
        let threshold = row[(node + offset) & row_mask];
        let degree = graph.external[node].len();
        let accept = match graph.partner[node] {
            None => mask_for_threshold(&planes, degree, threshold.external),
            Some(partner) => {
                let aligned = !(spins[node] ^ spins[partner]);
                let aligned_accept = mask_for_threshold(&planes, degree, threshold.aligned);
                let broken_accept = mask_for_threshold(&planes, degree, threshold.broken);
                (aligned & aligned_accept) | (!aligned & broken_accept)
            }
        };
        state.spins_mut()[node] ^= accept;
    }
}

fn pair_pass(graph: &FractionalGraph, row: &[Threshold], offset: usize, state: &mut MscState) {
    let row_mask = row.len() - 1;
    for (index, &(u, v)) in graph.pairs.iter().enumerate() {
        let degree = graph.external[u].len() + graph.external[v].len();
        let planes = count_pair_external(graph, u, v, state.spins());
        let threshold = row[(index + offset) & row_mask].external;
        let accept = mask_for_threshold(&planes, degree, threshold);
        let spins = state.spins_mut();
        spins[u] ^= accept;
        spins[v] ^= accept;
    }
}

fn sweep_offset(seed: u64, beta_index: usize, sweep: usize) -> usize {
    let mut x =
        (seed ^ ((beta_index as u64) << 20) ^ sweep as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((x ^ (x >> 31)) as usize) & (draw_row() - 1)
}

fn beta_ladder(graph: &IsingGraph, params: &SampleParams, seeded: bool) -> Vec<f64> {
    let cold_ladder = build_beta_schedule(graph, params);
    if seeded {
        let cold = *cold_ladder.last().expect("a beta ladder has one rung");
        vec![cold; cold_ladder.len()]
    } else {
        cold_ladder
    }
}

fn initial_states(nodes: usize, params: &SampleParams, seeds: Option<&[Vec<i8>]>) -> Vec<MscState> {
    let reads = params.num_reads.max(1);
    let seeds = seeds.unwrap_or(&[]);
    let mut states = Vec::with_capacity(reads.div_ceil(LANES));
    for read in (0..reads).step_by(LANES) {
        let mut rng = read_rng(params.seed, read);
        let word_seeds: Vec<&[i8]> = seeds
            .iter()
            .skip(read)
            .take(LANES)
            .map(Vec::as_slice)
            .collect();
        states.push(if word_seeds.is_empty() {
            MscState::random(nodes, &mut rng)
        } else {
            MscState::seeded(nodes, &word_seeds, &mut rng)
        });
    }
    states
}

/// Sample a zero-field graph with disjoint `-1.0` dimer bonds and fractional
/// external bonds. When `moves` is true, each single-spin pass is followed by
/// one pass of simultaneous dimer proposals using an independent random stream.
pub(crate) fn sample_fractional(
    graph: &IsingGraph,
    params: &SampleParams,
    moves: bool,
    seeds: Option<&[Vec<i8>]>,
) -> Result<Vec<Vec<i8>>, String> {
    let fractional = FractionalGraph::build(graph)?;
    let reads = params.num_reads.max(1);
    if let Some(seeds) = seeds {
        if seeds.len() > reads {
            return Err(format!("{} seeds exceed {reads} reads", seeds.len()));
        }
        for (index, seed) in seeds.iter().enumerate() {
            if seed.len() != graph.h.len() {
                return Err(format!(
                    "seed {index} has {} spins; expected {}",
                    seed.len(),
                    graph.h.len()
                ));
            }
            if seed.iter().any(|&spin| spin != -1 && spin != 1) {
                return Err(format!("seed {index} contains a spin other than -1 or +1"));
            }
        }
    }

    let betas = beta_ladder(graph, params, seeds.is_some());
    if betas.iter().any(|beta| !beta.is_finite() || *beta <= 0.0) {
        return Err("the beta ladder must contain finite positive values".to_owned());
    }
    let colors = Coloring::new(&CpuGraph::from_base(graph));
    let mut states = initial_states(graph.h.len(), params, seeds);
    let mut single_rng = SmallRng::seed_from_u64(params.seed ^ SINGLE_SALT);
    let mut pair_rng = SmallRng::seed_from_u64(params.seed ^ PAIR_SALT);
    let mut single_row = vec![Threshold::default(); draw_row()];
    let mut pair_row = if moves {
        vec![Threshold::default(); draw_row()]
    } else {
        Vec::new()
    };

    let sweeps_per_beta = effective_sweeps_per_beta(params.num_sweeps, params.sweeps_per_beta);
    for (beta_index, &beta) in betas.iter().enumerate() {
        fill_threshold_row(beta, fractional.other, &mut single_rng, &mut single_row);
        if moves {
            fill_threshold_row(beta, fractional.other, &mut pair_rng, &mut pair_row);
        }
        for sweep in 0..sweeps_per_beta {
            let offset = sweep_offset(params.seed, beta_index, sweep);
            for class in colors.classes() {
                for state in &mut states {
                    sweep_word(&fractional, &single_row, class, offset, state);
                }
            }
            if moves {
                let offset = sweep_offset(params.seed ^ PAIR_SALT, beta_index, sweep);
                for state in &mut states {
                    pair_pass(&fractional, &pair_row, offset, state);
                }
            }
        }
    }

    Ok((0..reads)
        .map(|read| states[read / LANES].lane(read % LANES))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packed_states(n: usize) -> (Vec<Vec<i8>>, MscState) {
        let seeds: Vec<Vec<i8>> = (0..LANES)
            .map(|lane| {
                (0..n)
                    .map(|node| if lane & (1 << node) == 0 { 1 } else { -1 })
                    .collect()
            })
            .collect();
        let refs: Vec<&[i8]> = seeds.iter().map(Vec::as_slice).collect();
        let mut rng = SmallRng::seed_from_u64(1);
        let state = MscState::seeded(n, &refs, &mut rng);
        (seeds, state)
    }

    fn half_delta(graph: &IsingGraph, spins: &[i8], flipped: &[usize]) -> f64 {
        graph
            .edges
            .iter()
            .zip(&graph.j)
            .map(|(&(u, v), &j)| {
                let before = j * f64::from(spins[u]) * f64::from(spins[v]);
                let su = if flipped.contains(&u) {
                    -spins[u]
                } else {
                    spins[u]
                };
                let sv = if flipped.contains(&v) {
                    -spins[v]
                } else {
                    spins[v]
                };
                (j * f64::from(su) * f64::from(sv) - before) / 2.0
            })
            .sum()
    }

    #[test]
    fn single_masks_match_scalar_deltas_for_both_coefficients() {
        for other in [0.031, 0.062] {
            let graph = IsingGraph::new(
                vec![0.0; 6],
                vec![-1.0, other, -other, other, -other],
                vec![(0, 1), (0, 2), (0, 3), (1, 4), (5, 2)],
            );
            let packed = FractionalGraph::build(&graph).expect("fractional graph");
            let (seeds, state) = packed_states(6);
            for node in [0, 1, 5] {
                let planes = count_external(&packed.external[node], node, state.spins());
                for m in -22..=22 {
                    let degree = packed.external[node].len();
                    let t = other * f64::from(m);
                    let threshold = Threshold {
                        aligned: signed_threshold(t, 1.0, other),
                        broken: signed_threshold(t, -1.0, other),
                        external: signed_threshold(t, 0.0, other),
                    };
                    let mask = match packed.partner[node] {
                        None => mask_for_threshold(&planes, degree, threshold.external),
                        Some(partner) => {
                            let aligned = !(state.spins()[node] ^ state.spins()[partner]);
                            let a = mask_for_threshold(&planes, degree, threshold.aligned);
                            let b = mask_for_threshold(&planes, degree, threshold.broken);
                            (aligned & a) | (!aligned & b)
                        }
                    };
                    for (lane, spins) in seeds.iter().enumerate() {
                        assert_eq!(
                            mask & (1 << lane) != 0,
                            half_delta(&graph, spins, &[node]) <= t,
                            "other={other}, node={node}, lane={lane}, m={m}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn repaired_thresholds_include_exact_boundaries() {
        for other in [0.031, 0.062] {
            for offset in [-1.0, 0.0, 1.0] {
                for m in -38..=38 {
                    let boundary = other * f64::from(m) + offset;
                    assert_eq!(signed_threshold(boundary, offset, other), m);
                    let below = if boundary > 0.0 {
                        f64::from_bits(boundary.to_bits() - 1)
                    } else if boundary < 0.0 {
                        f64::from_bits(boundary.to_bits() + 1)
                    } else {
                        -f64::from_bits(1)
                    };
                    assert!(signed_threshold(below, offset, other) < m);
                }
            }
        }
    }

    #[test]
    fn pair_masks_exclude_the_internal_bond() {
        for other in [0.031, 0.062] {
            let graph = IsingGraph::new(
                vec![0.0; 6],
                vec![-1.0, other, -other, other, -other],
                vec![(0, 1), (0, 2), (0, 3), (1, 4), (1, 5)],
            );
            let packed = FractionalGraph::build(&graph).expect("fractional graph");
            let (seeds, state) = packed_states(6);
            let planes = count_pair_external(&packed, 0, 1, state.spins());
            let degree = packed.external[0].len() + packed.external[1].len();
            for m in -(degree as i16)..=degree as i16 {
                let mask = mask_for_threshold(&planes, degree, m);
                let t = other * f64::from(m);
                for (lane, spins) in seeds.iter().enumerate() {
                    assert_eq!(
                        mask & (1 << lane) != 0,
                        half_delta(&graph, spins, &[0, 1]) <= t,
                        "other={other}, lane={lane}, m={m}"
                    );
                }
            }
        }
    }

    #[test]
    fn seeded_ladder_stays_at_cold_and_seeded_lanes_are_preserved_at_start() {
        let graph = IsingGraph::new(vec![0.0; 3], vec![-1.0, 0.031], vec![(0, 1), (1, 2)]);
        let params = SampleParams {
            num_reads: 65,
            num_sweeps: 12,
            sweeps_per_beta: 4,
            beta_range: Some((0.25, 8.0)),
            seed: 17,
        };
        let ladder = beta_ladder(&graph, &params, true);
        assert_eq!(ladder.len(), 3);
        assert!(ladder.iter().all(|&beta| beta == ladder[0]));
        assert!((ladder[0] - 8.0).abs() < 1e-12);
        let seeds = vec![vec![-1, 1, -1], vec![1, -1, 1]];
        let states = initial_states(3, &params, Some(&seeds));
        assert_eq!(states.len(), 2);
        assert_eq!(states[0].lane(0), seeds[0]);
        assert_eq!(states[0].lane(1), seeds[1]);

        let mut expected_rng = read_rng(params.seed, 64);
        let expected = MscState::random(3, &mut expected_rng);
        assert_eq!(states[1].lane(0), expected.lane(0));

        let samples = sample_fractional(&graph, &params, true, Some(&seeds))
            .expect("seeded fractional sample");
        assert_eq!(samples.len(), params.num_reads);
        assert!(samples
            .iter()
            .flatten()
            .all(|&spin| spin == -1 || spin == 1));
    }
}
