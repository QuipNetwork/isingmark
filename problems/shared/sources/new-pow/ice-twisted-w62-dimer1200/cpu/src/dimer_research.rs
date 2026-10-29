//! Exact packed sampling and aligned-cluster research prototypes.

use std::collections::BTreeMap;
use std::time::Instant;

use quip_solver_core::{IsingGraph, SampleParams};

use crate::coloring::Coloring;
use crate::research::validate_common;
use crate::sa_int::IntGraph;
use crate::sa_msc::{
    anneal_words_with_clusters, bond_counts, ClusterMove, MscState, LANES, MAX_DEGREE,
};
use crate::sa_msc_f64::energy_f64;
use crate::sa_sampler::{read_rng, sample_unit_packed, SeededStart};
use crate::sampler_core::{build_beta_schedule, CpuGraph};

/// Variants measured by the local dimer experiment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DimerVariant {
    /// Existing unit MSA on an exact integer representation.
    BaselineUnit,
    /// The same single-spin pass followed by simultaneous dimer moves.
    PairUnit,
    /// A single-spin pass followed by simultaneous three- or four-spin moves.
    ClusterUnit,
    /// Existing unit MSA on the contracted graph, then expansion.
    ReducedUnit,
    /// Seven eighths contraction and one eighth cold full-graph polish.
    ReducedPolishUnit,
    /// Exact milliweight single-spin updates with per-lane cached fields.
    BaselineWeighted,
    /// Weighted singles followed by pair flips.
    PairWeighted,
    /// Weighted singles followed by whole-cluster flips.
    ClusterWeighted,
    /// Weighted singles, cluster flips, and exact conditional block minima.
    BlockWeighted,
    /// Weighted singles and sequential exact updates of overlapping local blocks.
    LocalBlockWeighted,
    /// Local block updates followed by exact minima on signed strong paths.
    PathBlockWeighted,
    /// Local tetrahedron updates followed by an alternating ice loop or boundary string.
    IceLoopWeighted,
    /// Local tetrahedron updates followed by 64 ice attempts per read per sweep.
    IceLoop64Weighted,
    /// Local tetrahedra with 32 loop attempts and 32 defect-path attempts per read.
    IceDefect64Weighted,
    /// Two temperature ladders with full-graph disagreement components.
    PtIceFullWeighted,
    /// Two temperature ladders with Metropolis-tested strong components.
    PtIceStrongWeighted,
    /// Fixed geometric temperatures with full-graph disagreement components.
    PtIceGeometricFullWeighted,
    /// Fixed geometric temperatures with Metropolis-tested strong components.
    PtIceGeometricStrongWeighted,
    /// Fixed full-component control with energy-window diagnostics.
    PtIceProfileFullWeighted,
    /// Fixed strong-component control with energy-window diagnostics.
    PtIceProfileStrongWeighted,
    /// Fixed band with full disagreement moves and a counted cold ice descent.
    PtIceBandFullWeighted,
    /// Fixed band with strong disagreement moves and a counted cold ice descent.
    PtIceBandStrongWeighted,
    /// Contract ferromagnetic dimers and solve with the weighted kernel.
    ReducedWeighted,
    /// Contract, solve, expand, and cold-polish with exact milliweights.
    ReducedPolishWeighted,
}

impl DimerVariant {
    /// Stable variant label in the experiment JSONL.
    pub fn name(self) -> &'static str {
        match self {
            Self::BaselineUnit => "baseline-unit",
            Self::PairUnit => "pair-unit",
            Self::ClusterUnit => "cluster-unit",
            Self::ReducedUnit => "reduced-unit",
            Self::ReducedPolishUnit => "reduced-polish-unit",
            Self::BaselineWeighted => "baseline-weighted",
            Self::PairWeighted => "pair-weighted",
            Self::ClusterWeighted => "cluster-weighted",
            Self::BlockWeighted => "block-weighted",
            Self::LocalBlockWeighted => "local-block-weighted",
            Self::PathBlockWeighted => "path-block-weighted",
            Self::IceLoopWeighted => "ice-loop-weighted",
            Self::IceLoop64Weighted => "ice-loop64-weighted",
            Self::IceDefect64Weighted => "ice-defect64-weighted",
            Self::PtIceFullWeighted => "pt-ice-full-weighted",
            Self::PtIceStrongWeighted => "pt-ice-strong-weighted",
            Self::PtIceGeometricFullWeighted => "pt-ice-geometric-full-weighted",
            Self::PtIceGeometricStrongWeighted => "pt-ice-geometric-strong-weighted",
            Self::PtIceProfileFullWeighted => "pt-ice-profile-full-weighted",
            Self::PtIceProfileStrongWeighted => "pt-ice-profile-strong-weighted",
            Self::PtIceBandFullWeighted => "pt-ice-band-full-weighted",
            Self::PtIceBandStrongWeighted => "pt-ice-band-strong-weighted",
            Self::ReducedWeighted => "reduced-weighted",
            Self::ReducedPolishWeighted => "reduced-polish-weighted",
        }
    }
}

/// Energy moments before exchanges, indexed by ladder copy and temperature.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EnergyWindow {
    /// Zero-based first sweep in the window.
    pub first_sweep: usize,
    /// Number of observations per copy and temperature.
    pub sweeps: usize,
    /// Mean energy in physical energy units.
    pub mean_energy_units: [[f64; 32]; 2],
    /// Population temporal variance in squared physical energy units.
    pub variance_energy_units_squared: [[f64; 32]; 2],
}

/// Temperature exchange and replica-move diagnostics.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct TemperingStats {
    /// Nonoverlapping diagnostic windows, absent from ordinary solver output.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub energy_windows: Vec<EnergyWindow>,
    /// Fixed ladder used after the counted warmup.
    pub final_betas: [f64; 32],
    /// Warmup sweeps included in the total solve budget.
    pub warmup_sweeps: usize,
    /// Temperature adaptations within warmup.
    pub ladder_updates: u64,
    /// Adjacent exchanges proposed across both copies.
    pub swap_attempts: [u64; 31],
    /// Adjacent exchanges accepted across both copies.
    pub swap_accepted: [u64; 31],
    /// Proposals after the ladder freezes.
    pub frozen_swap_attempts: [u64; 31],
    /// Accepted exchanges after the ladder freezes.
    pub frozen_swap_accepted: [u64; 31],
    /// Hot-to-cold-to-hot trips by physical replica.
    pub round_trips: [[u64; 32]; 2],
    /// Temperature round trips after the ladder freezes.
    pub frozen_round_trips: [[u64; 32]; 2],
    /// Paired disagreement proposals, including identical states.
    pub cluster_attempts: u64,
    /// Nonempty components before acceptance.
    pub cluster_formed: u64,
    /// Accepted paired flips.
    pub cluster_accepted: u64,
    /// Sites in all formed components, before acceptance.
    pub cluster_proposed_sites: u64,
    /// Sites in accepted components, counted once for the pair.
    pub cluster_accepted_sites: u64,
    /// Largest proposed component.
    pub cluster_max_sites: usize,
    /// Formed components with nonzero joint full-model delta.
    pub nonzero_joint_delta: u64,
    /// Global inversions that reduced disagreement size.
    pub gauge_flips: u64,
    /// Exact starting energies indexed by physical replica.
    pub initial_energies_milli: [[i64; 32]; 2],
    /// Exact final energies indexed by physical replica.
    pub final_energies_milli: [[i64; 32]; 2],
}

/// Counts across all requested reads and sweeps in the ice attacks.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct IceLoopStats {
    /// Replica statistics, absent from legacy ice output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tempering: Option<TemperingStats>,
    /// Random starts, including paths that end at defects or unmatched boundaries.
    pub attempts: u64,
    /// Valid closed alternating paths before the energy test.
    pub formed_loops: u64,
    /// Valid boundary-to-boundary alternating paths before the energy test.
    pub formed_strings: u64,
    /// Proposals accepted by the full-model energy test.
    pub accepted: u64,
    /// Total spin flips in accepted paths.
    pub flipped_spins: u64,
    /// Uniform tetrahedron starts reserved for defect paths.
    #[serde(skip_serializing_if = "is_zero")]
    pub defect_attempts: u64,
    /// Valid paths with at least two spins starting at a defect.
    #[serde(skip_serializing_if = "is_zero")]
    pub formed_defects: u64,
    /// Defect paths accepted by the full-model energy test.
    #[serde(skip_serializing_if = "is_zero")]
    pub accepted_defects: u64,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// All states and stage costs, retained for independent external scoring.
pub struct DimerSamples {
    /// Final states, or the better seed/final state in each polished lane.
    pub spins: Vec<Vec<i8>>,
    /// Expanded states before polish, when applicable.
    pub expanded_spins: Option<Vec<Vec<i8>>>,
    /// Raw polish output before retaining each lane's incumbent.
    pub final_spins: Option<Vec<Vec<i8>>>,
    /// Elapsed solve time, including graph conversion.
    pub solve_seconds: f64,
    /// Elapsed polish time, including full-graph conversion and seed selection.
    pub polish_seconds: f64,
    /// Sweeps in the main solve.
    pub solve_sweeps: usize,
    /// Sweeps in the full-graph polish.
    pub polish_sweeps: usize,
    /// Ice move counts, present only for the ice variants.
    pub ice_loop_stats: Option<IceLoopStats>,
}

fn clusters(graph: &IsingGraph) -> Result<Vec<Vec<usize>>, String> {
    if graph.h.iter().any(|&h| h != 0.0) {
        return Err("the dimer prototype requires zero fields".to_owned());
    }
    let strongest = graph.j.iter().map(|j| j.abs()).fold(0.0, f64::max);
    if strongest == 0.0 {
        return Err("the model must contain a nonzero dimer bond".to_owned());
    }
    let mut neighbors = vec![Vec::new(); graph.h.len()];
    for (&(u, v), &j) in graph.edges.iter().zip(&graph.j) {
        if j == -strongest {
            neighbors[u].push(v);
            neighbors[v].push(u);
        }
    }
    let mut groups = Vec::new();
    let mut seen = vec![false; graph.h.len()];
    for start in 0..seen.len() {
        if seen[start] || neighbors[start].is_empty() {
            continue;
        }
        let mut group = Vec::new();
        let mut pending = vec![start];
        seen[start] = true;
        while let Some(node) = pending.pop() {
            group.push(node);
            for &next in &neighbors[node] {
                if !seen[next] {
                    seen[next] = true;
                    pending.push(next);
                }
            }
        }
        group.sort_unstable();
        groups.push(group);
    }
    if groups.is_empty() {
        return Err("the model must contain a ferromagnetic cluster".to_owned());
    }
    Ok(groups)
}

fn unit_encoding(graph: &IsingGraph) -> Result<(IsingGraph, f64), String> {
    let unit = graph
        .j
        .iter()
        .map(|j| j.abs())
        .filter(|&j| j > 0.0)
        .reduce(f64::min)
        .unwrap_or(1.0);
    let scale = unit.recip();
    if !scale.is_finite() || scale <= 0.0 {
        return Err("coupling unit has no finite positive reciprocal".to_owned());
    }
    let mut edges = Vec::new();
    let mut couplings = Vec::new();
    for (&edge, &j) in graph.edges.iter().zip(&graph.j) {
        let scaled = scale * j;
        let rounded = scaled.round();
        if !scaled.is_finite()
            || (scaled - rounded).abs() > 1e-10
            || rounded.abs() > MAX_DEGREE as f64
        {
            return Err(format!(
                "coefficient {j} is not representable at scale {scale}"
            ));
        }
        for _ in 0..rounded.abs() as usize {
            edges.push(edge);
            couplings.push(scaled.signum());
        }
    }
    Ok((
        IsingGraph::new(vec![0.0; graph.h.len()], couplings, edges),
        scale,
    ))
}

pub(crate) fn contract(
    graph: &IsingGraph,
    clusters: &[Vec<usize>],
) -> (IsingGraph, Vec<usize>, f64) {
    let mut sites = vec![usize::MAX; graph.h.len()];
    for (site, nodes) in clusters.iter().enumerate() {
        for &node in nodes {
            sites[node] = site;
        }
    }
    let mut num_sites = clusters.len();
    for site in &mut sites {
        if *site == usize::MAX {
            *site = num_sites;
            num_sites += 1;
        }
    }
    let mut sums = BTreeMap::new();
    let mut offset = 0.0;
    for (&(u, v), &j) in graph.edges.iter().zip(&graph.j) {
        let (a, b) = (sites[u], sites[v]);
        if a == b {
            offset += j;
        } else {
            *sums.entry((a.min(b), a.max(b))).or_insert(0.0) += j;
        }
    }
    let mut edges = Vec::new();
    let mut couplings = Vec::new();
    for (edge, j) in sums {
        if j != 0.0 {
            edges.push(edge);
            couplings.push(j);
        }
    }
    (
        IsingGraph::new(vec![0.0; num_sites], couplings, edges),
        sites,
        offset,
    )
}

fn sample_encoded(
    graph: &IsingGraph,
    params: &SampleParams,
    clusters: &[Vec<usize>],
    seeds: Option<&[Vec<i8>]>,
) -> Result<Vec<Vec<i8>>, String> {
    if graph.j.contains(&-1.0) && graph.j.iter().any(|j| j.abs() == 0.031 || j.abs() == 0.062) {
        return crate::dimer_fractional::sample_fractional(
            graph,
            params,
            !clusters.is_empty(),
            seeds,
        );
    }
    let (encoded, scale) = unit_encoding(graph)?;
    let int = IntGraph::from_base(&encoded).ok_or("encoded graph exceeds integer bounds")?;
    let counts = bond_counts(&int).ok_or("encoded degree exceeds the unit kernel bound")?;
    let (hot, cold) = params
        .beta_range
        .ok_or("an explicit beta range is required")?;
    let scaled_params = SampleParams {
        beta_range: Some((hot / scale, cold / scale)),
        ..params.clone()
    };
    if clusters.is_empty() {
        let start = seeds.map(|spins| SeededStart {
            spins,
            start_beta: Some(cold / scale),
        });
        let (samples, _) = sample_unit_packed(&encoded, &scaled_params, &int, &counts, start, None)
            .map_err(|e| format!("unit sampling stopped: {e:?}"))?;
        return Ok(samples.into_iter().map(|r| r.spins).collect());
    }
    let moves = ClusterMove::build(&int, clusters)
        .ok_or("cluster neighborhoods exceed the supported bound")?;
    let colors = Coloring::new(&CpuGraph::from_base(&encoded));
    let betas = build_beta_schedule(&encoded, &scaled_params);
    let mut states = Vec::new();
    for read in (0..params.num_reads).step_by(LANES) {
        states.push(MscState::random(
            graph.h.len(),
            &mut read_rng(params.seed, read),
        ));
    }
    anneal_words_with_clusters(
        &int,
        &counts,
        &colors,
        &betas,
        &scaled_params,
        &mut states,
        None,
        &moves,
    )
    .map_err(|e| format!("pair sampling stopped: {e:?}"))?;
    Ok((0..params.num_reads)
        .map(|read| states[read / LANES].lane(read % LANES))
        .collect())
}

/// Run one dimer experiment on one CPU thread, returning every lane.
///
/// Detect connected components of the largest negative bonds. For an explicit
/// cluster assignment, including equal-strength ladders, use [`sample_clusters`].
///
/// # Errors
/// Rejects malformed graphs, unsupported coefficients/degrees, absent beta
/// endpoints, and sweep budgets that the existing beta ladder would round.
pub fn sample_dimer(
    graph: &IsingGraph,
    params: &SampleParams,
    variant: DimerVariant,
) -> Result<DimerSamples, String> {
    validate_common(graph, params, None).map_err(|e| e.to_string())?;
    let groups = clusters(graph)?;
    sample_clusters(graph, params, variant, &groups)
}

/// Sample a zero-field graph with variant-specific local groups.
/// Unit variants require disjoint groups of two through four spins connected
/// by negative bonds. Weighted partition variants accept signed strong
/// components of two through eight spins. Pair variants require two-spin groups,
/// and weighted contraction additionally requires ferromagnetic dimers.
/// `LocalBlockWeighted` accepts overlapping groups of two through eight distinct
/// spins connected by any nonzero bonds. Updates visit these groups sequentially.
/// `PathBlockWeighted` adds exact updates of induced signed 1,000-milli paths.
/// It rejects strong branches, strong cycles, and weak chords within those paths.
/// The ice variants require edge-disjoint tetrahedra whose six 1,000-milli
/// bonds admit an antiferromagnetic gauge within each tetrahedron. A spin can belong
/// to at most two tetrahedra.
/// Spins omitted from the assignment retain their individual sites and updates.
///
/// # Errors
/// Rejects invalid assignments, nonzero fields, unsupported neighborhoods,
/// absent beta endpoints, and sweep budgets that would be rounded.
pub fn sample_clusters(
    graph: &IsingGraph,
    params: &SampleParams,
    variant: DimerVariant,
    groups: &[Vec<usize>],
) -> Result<DimerSamples, String> {
    match variant {
        DimerVariant::PtIceFullWeighted | DimerVariant::PtIceStrongWeighted => {
            return crate::dimer_weighted::tempering::sample(
                graph,
                params,
                groups,
                variant == DimerVariant::PtIceStrongWeighted,
                crate::dimer_weighted::tempering::Ladder::Adaptive,
            );
        }
        DimerVariant::PtIceGeometricFullWeighted | DimerVariant::PtIceGeometricStrongWeighted => {
            return crate::dimer_weighted::tempering::sample(
                graph,
                params,
                groups,
                variant == DimerVariant::PtIceGeometricStrongWeighted,
                crate::dimer_weighted::tempering::Ladder::Geometric,
            );
        }
        DimerVariant::PtIceProfileFullWeighted | DimerVariant::PtIceProfileStrongWeighted => {
            return crate::dimer_weighted::tempering::sample(
                graph,
                params,
                groups,
                variant == DimerVariant::PtIceProfileStrongWeighted,
                crate::dimer_weighted::tempering::Ladder::ProfileGeometric,
            );
        }
        DimerVariant::PtIceBandFullWeighted | DimerVariant::PtIceBandStrongWeighted => {
            return crate::dimer_weighted::tempering::sample_band(
                graph,
                params,
                groups,
                variant == DimerVariant::PtIceBandStrongWeighted,
            );
        }
        DimerVariant::BaselineWeighted
        | DimerVariant::PairWeighted
        | DimerVariant::ClusterWeighted
        | DimerVariant::BlockWeighted
        | DimerVariant::LocalBlockWeighted
        | DimerVariant::PathBlockWeighted
        | DimerVariant::IceLoopWeighted
        | DimerVariant::IceLoop64Weighted
        | DimerVariant::IceDefect64Weighted
        | DimerVariant::ReducedWeighted
        | DimerVariant::ReducedPolishWeighted => {
            return crate::dimer_weighted::sample(graph, params, variant, groups);
        }
        DimerVariant::BaselineUnit
        | DimerVariant::PairUnit
        | DimerVariant::ClusterUnit
        | DimerVariant::ReducedUnit
        | DimerVariant::ReducedPolishUnit => {}
    }
    sample_aligned_clusters(graph, params, variant, groups)
}

fn sample_aligned_clusters(
    graph: &IsingGraph,
    params: &SampleParams,
    variant: DimerVariant,
    groups: &[Vec<usize>],
) -> Result<DimerSamples, String> {
    let started = Instant::now();
    validate_common(graph, params, None).map_err(|e| e.to_string())?;
    if params.beta_range.is_none() {
        return Err("an explicit beta range is required".to_owned());
    }
    validate_clusters(graph, groups)?;
    if variant == DimerVariant::PairUnit && groups.iter().any(|group| group.len() != 2) {
        return Err("pair flips require size-two clusters".to_owned());
    }
    if variant == DimerVariant::ReducedPolishUnit && !params.num_sweeps.is_multiple_of(8) {
        return Err("polished sweep budgets must be divisible by eight".to_owned());
    }
    let polish_sweeps = if variant == DimerVariant::ReducedPolishUnit {
        params.num_sweeps / 8
    } else {
        0
    };
    let solve_sweeps = params.num_sweeps - polish_sweeps;
    let per_beta = params.sweeps_per_beta.max(1);
    if !solve_sweeps.is_multiple_of(per_beta)
        || (variant == DimerVariant::ReducedPolishUnit
            && (polish_sweeps == 0 || !polish_sweeps.is_multiple_of(per_beta)))
    {
        return Err("each stage must contain whole sweeps_per_beta blocks".to_owned());
    }
    let solve_params = SampleParams {
        num_sweeps: solve_sweeps,
        ..params.clone()
    };
    let mut spins = match variant {
        DimerVariant::BaselineUnit => sample_encoded(graph, &solve_params, &[], None)?,
        DimerVariant::PairUnit | DimerVariant::ClusterUnit => {
            sample_encoded(graph, &solve_params, groups, None)?
        }
        DimerVariant::ReducedUnit | DimerVariant::ReducedPolishUnit => {
            let (reduced, sites, _) = contract(graph, groups);
            let reduced_states = sample_encoded(&reduced, &solve_params, &[], None)?;
            reduced_states
                .iter()
                .map(|s| sites.iter().map(|&site| s[site]).collect())
                .collect()
        }
        DimerVariant::PtIceFullWeighted
        | DimerVariant::PtIceStrongWeighted
        | DimerVariant::PtIceGeometricFullWeighted
        | DimerVariant::PtIceGeometricStrongWeighted
        | DimerVariant::PtIceProfileFullWeighted
        | DimerVariant::PtIceProfileStrongWeighted
        | DimerVariant::PtIceBandFullWeighted
        | DimerVariant::PtIceBandStrongWeighted
        | DimerVariant::BaselineWeighted
        | DimerVariant::PairWeighted
        | DimerVariant::ClusterWeighted
        | DimerVariant::BlockWeighted
        | DimerVariant::LocalBlockWeighted
        | DimerVariant::PathBlockWeighted
        | DimerVariant::IceLoopWeighted
        | DimerVariant::IceLoop64Weighted
        | DimerVariant::IceDefect64Weighted
        | DimerVariant::ReducedWeighted
        | DimerVariant::ReducedPolishWeighted => {
            return Err("weighted variants require the weighted sampler".to_owned());
        }
    };
    let solve_seconds = started.elapsed().as_secs_f64();
    let mut expanded_spins = None;
    let mut final_spins = None;
    let mut polish_seconds = 0.0;
    if polish_sweeps > 0 {
        let started = Instant::now();
        let polish_params = SampleParams {
            num_sweeps: polish_sweeps,
            seed: params.seed ^ 0x504F_4C49_5348_3031,
            ..params.clone()
        };
        let polished = sample_encoded(graph, &polish_params, &[], Some(&spins))?;
        expanded_spins = Some(spins.clone());
        // Milli-valued fractional inputs need integer comparison on ties:
        // summation noise must not replace an equal-energy incumbent.
        let milli = graph
            .j
            .iter()
            .all(|&j| [-1.0, -0.031, 0.031, -0.062, 0.062].contains(&j))
            .then(|| {
                graph
                    .j
                    .iter()
                    .map(|j| (j * 1000.0).round() as i64)
                    .collect::<Vec<_>>()
            });
        let comparison_energy = |state: &[i8]| match &milli {
            Some(couplings) => graph
                .edges
                .iter()
                .zip(couplings)
                .map(|(&(u, v), &j)| j * i64::from(state[u]) * i64::from(state[v]))
                .sum::<i64>() as f64,
            None => energy_f64(state, graph),
        };
        for (incumbent, candidate) in spins.iter_mut().zip(&polished) {
            if comparison_energy(candidate) < comparison_energy(incumbent) {
                incumbent.clone_from(candidate);
            }
        }
        final_spins = Some(polished);
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
        ice_loop_stats: None,
    })
}

fn validate_clusters(graph: &IsingGraph, groups: &[Vec<usize>]) -> Result<(), String> {
    if graph.h.iter().any(|&h| h != 0.0) {
        return Err("cluster sampling requires zero fields".to_owned());
    }
    let mut sites = vec![usize::MAX; graph.h.len()];
    let mut internal = vec![Vec::new(); groups.len()];
    for (site, nodes) in groups.iter().enumerate() {
        if !(2..=4).contains(&nodes.len()) {
            return Err("clusters must contain two through four spins".to_owned());
        }
        for &node in nodes {
            if node >= sites.len() || sites[node] != usize::MAX {
                return Err("cluster sites must be valid and disjoint".to_owned());
            }
            sites[node] = site;
        }
    }
    for (&(u, v), &j) in graph.edges.iter().zip(&graph.j) {
        if sites[u] != usize::MAX && sites[u] == sites[v] {
            if j >= 0.0 {
                return Err("internal cluster bonds must be negative".to_owned());
            }
            internal[sites[u]].push((u, v));
        }
    }
    for (nodes, edges) in groups.iter().zip(internal) {
        let mut reached = vec![nodes[0]];
        for _ in 1..nodes.len() {
            for &(u, v) in &edges {
                if reached.contains(&u) && !reached.contains(&v) {
                    reached.push(v);
                }
                if reached.contains(&v) && !reached.contains(&u) {
                    reached.push(u);
                }
            }
        }
        if reached.len() != nodes.len() {
            return Err("each cluster must be connected".to_owned());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sa_msc_f64::energy_f64;

    fn two_dimers() -> IsingGraph {
        IsingGraph::new(
            vec![0.0; 4],
            vec![-2.0, -2.0, 1.0, -0.5, -0.5],
            vec![(0, 2), (1, 3), (0, 1), (0, 3), (2, 1)],
        )
    }

    #[test]
    fn repeated_unit_edges_preserve_every_state_energy_at_scale_two() {
        let graph = two_dimers();
        let (encoded, scale) = unit_encoding(&graph).expect("half-integer model");
        assert_eq!(scale, 2.0);
        assert!(encoded.j.iter().all(|j| j.abs() == 1.0));
        for bits in 0..16 {
            let spins: Vec<i8> = (0..4)
                .map(|i| if bits & (1 << i) == 0 { 1 } else { -1 })
                .collect();
            assert_eq!(
                energy_f64(&spins, &encoded),
                2.0 * energy_f64(&spins, &graph)
            );
        }
    }

    #[test]
    fn contraction_sums_parallel_bonds_and_preserves_expanded_energy() {
        let graph = two_dimers();
        let (reduced, sites, offset) = contract(&graph, &[vec![0, 2], vec![1, 3]]);
        assert_eq!(reduced.h.len(), 2);
        assert!(reduced.edges.is_empty(), "opposite couplings cancel");
        assert_eq!(offset, -4.0);
        for bits in 0..4 {
            let spins: Vec<i8> = (0..2)
                .map(|i| if bits & (1 << i) == 0 { 1 } else { -1 })
                .collect();
            let expanded: Vec<i8> = sites.iter().map(|&site| spins[site]).collect();
            assert_eq!(
                energy_f64(&expanded, &graph),
                energy_f64(&spins, &reduced) + offset
            );
        }
    }

    #[test]
    fn prototype_baseline_is_the_existing_kernel_with_scaled_beta() {
        use crate::{SaSampler, SaVariant, Sampler};
        let graph = two_dimers();
        let params = SampleParams {
            num_reads: 65,
            num_sweeps: 64,
            sweeps_per_beta: 4,
            beta_range: Some((0.1, 4.0)),
            seed: 19,
        };
        let actual = sample_dimer(&graph, &params, DimerVariant::BaselineUnit).expect("baseline");
        let (encoded, _) = unit_encoding(&graph).expect("encoding");
        let scaled = SampleParams {
            beta_range: Some((0.05, 2.0)),
            ..params
        };
        let expected = SaSampler::new(SaVariant::MultiSpin)
            .sample(&encoded, &scaled)
            .expect("existing kernel");
        assert_eq!(
            actual.spins,
            expected.into_iter().map(|r| r.spins).collect::<Vec<_>>()
        );
    }

    #[test]
    fn reduced_polish_retains_incumbents_and_accounts_for_all_sweeps() {
        let graph = two_dimers();
        let params = SampleParams {
            num_reads: 65,
            num_sweeps: 64,
            sweeps_per_beta: 4,
            beta_range: Some((0.1, 0.2)),
            seed: 7,
        };
        let out = sample_dimer(&graph, &params, DimerVariant::ReducedPolishUnit).expect("polish");
        assert_eq!(out.solve_sweeps + out.polish_sweeps, 64);
        let seeds = out.expanded_spins.expect("expanded");
        let finals = out.final_spins.expect("finals");
        assert_eq!(out.spins.len(), 65);
        for ((chosen, seed), final_state) in out.spins.iter().zip(&seeds).zip(&finals) {
            assert_eq!(seed[0], seed[2]);
            assert_eq!(seed[1], seed[3]);
            assert_eq!(
                energy_f64(chosen, &graph),
                energy_f64(seed, &graph).min(energy_f64(final_state, &graph))
            );
        }
    }

    #[test]
    fn unsupported_dimer_inputs_are_refused_instead_of_falling_back() {
        let params = SampleParams {
            num_reads: 64,
            num_sweeps: 64,
            sweeps_per_beta: 4,
            beta_range: Some((0.1, 4.0)),
            seed: 19,
        };
        for graph in [
            IsingGraph::new(vec![0.0; 2], vec![2.0], vec![(0, 1)]),
            IsingGraph::new(vec![0.0; 3], vec![-2.0, -2.0], vec![(0, 1), (1, 2)]),
            IsingGraph::new(
                vec![0.0; 4],
                vec![-2.0, -2.0, 0.3],
                vec![(0, 2), (1, 3), (0, 1)],
            ),
            IsingGraph::new(vec![1.0, 0.0], vec![-2.0], vec![(0, 1)]),
        ] {
            assert!(sample_dimer(&graph, &params, DimerVariant::PairUnit).is_err());
        }
        let rounded_budget = SampleParams {
            num_sweeps: 68,
            ..params
        };
        assert!(sample_dimer(
            &two_dimers(),
            &rounded_budget,
            DimerVariant::ReducedPolishUnit
        )
        .is_err());
    }

    fn partial_native_graph() -> IsingGraph {
        IsingGraph::new(
            vec![0.0; 5],
            vec![-1.0, 0.125, -0.125, 0.125, -0.125],
            vec![(0, 1), (0, 2), (1, 2), (2, 3), (3, 4)],
        )
    }

    #[test]
    fn native_matching_accepts_unpaired_spins() {
        assert_eq!(
            clusters(&partial_native_graph()).expect("partial matching"),
            vec![vec![0, 1]]
        );
    }

    #[test]
    fn native_contraction_keeps_each_singleton_and_preserves_energy() {
        let graph = partial_native_graph();
        let (reduced, sites, offset) = contract(&graph, &[vec![0, 1]]);
        assert_eq!(reduced.h.len(), 4);
        assert_eq!(sites, vec![0, 0, 1, 2, 3]);
        assert_eq!(offset, -1.0);
        for bits in 0..16 {
            let spins: Vec<i8> = (0..4)
                .map(|i| if bits & (1 << i) == 0 { 1 } else { -1 })
                .collect();
            let expanded: Vec<i8> = sites.iter().map(|&site| spins[site]).collect();
            assert_eq!(
                energy_f64(&expanded, &graph),
                energy_f64(&spins, &reduced) + offset
            );
        }
    }

    #[test]
    fn native_variants_preserve_scaling_and_seeded_states() {
        use crate::{SaSampler, SaVariant, Sampler};
        let graph = partial_native_graph();
        let params = SampleParams {
            num_reads: 65,
            num_sweeps: 64,
            sweeps_per_beta: 4,
            beta_range: Some((0.1, 8.0)),
            seed: 19,
        };
        let mut edges = vec![(0, 1); 8];
        edges.extend_from_slice(&[(0, 2), (1, 2), (2, 3), (3, 4)]);
        let mut couplings = vec![-1.0; 8];
        couplings.extend_from_slice(&[1.0, -1.0, 1.0, -1.0]);
        let encoded = IsingGraph::new(vec![0.0; 5], couplings, edges);
        let scaled = SampleParams {
            beta_range: Some((0.0125, 1.0)),
            ..params
        };
        let expected = SaSampler::new(SaVariant::MultiSpin)
            .sample(&encoded, &scaled)
            .expect("unit kernel");
        let actual =
            sample_dimer(&graph, &params, DimerVariant::BaselineUnit).expect("native baseline");
        assert_eq!(
            actual.spins,
            expected.into_iter().map(|r| r.spins).collect::<Vec<_>>()
        );
        for variant in [
            DimerVariant::PairUnit,
            DimerVariant::ReducedUnit,
            DimerVariant::ReducedPolishUnit,
        ] {
            let result = sample_dimer(&graph, &params, variant).expect("native variant");
            assert_eq!(result.spins.len(), 65);
            assert!(result.spins.iter().all(|s| s.len() == 5));
            if variant == DimerVariant::ReducedUnit {
                assert!(result.spins.iter().all(|s| s[0] == s[1]));
            }
            if let (Some(seeds), Some(finals)) = (result.expanded_spins, result.final_spins) {
                for ((chosen, seed), final_state) in result.spins.iter().zip(&seeds).zip(&finals) {
                    assert_eq!(seed[0], seed[1]);
                    assert_eq!(
                        energy_f64(chosen, &graph),
                        energy_f64(seed, &graph).min(energy_f64(final_state, &graph))
                    );
                }
            }
        }
    }

    #[test]
    fn connected_triangles_and_four_spin_clusters_contract_with_singletons() {
        for size in [3, 4] {
            let mut edges = vec![(0, 1), (1, 2), (0, 2)];
            if size == 4 {
                edges.push((2, 3));
            }
            let mut j = vec![-1.0; edges.len()];
            edges.extend([(0, size), (1, size), (size - 1, size + 1)]);
            j.extend([0.125, -0.125, 0.125]);
            let graph = IsingGraph::new(vec![0.0; size + 2], j, edges);
            let params = SampleParams {
                num_reads: 64,
                num_sweeps: 32,
                sweeps_per_beta: 4,
                beta_range: Some((0.1, 8.0)),
                seed: 17,
            };
            let sampled = sample_dimer(&graph, &params, DimerVariant::ReducedUnit)
                .expect("connected clusters must contract");
            for state in sampled.spins {
                assert!(state[..size].iter().all(|spin| *spin == state[0]));
                assert_eq!(state.len(), size + 2);
            }
        }
    }

    #[test]
    fn explicit_matching_resolves_equal_strength_external_bonds() {
        let graph = IsingGraph::new(
            vec![0.0; 6],
            vec![-1.0, -1.0, -1.0, -1.0, 1.0],
            vec![(0, 1), (2, 3), (1, 2), (3, 4), (0, 5)],
        );
        let params = SampleParams {
            num_reads: 64,
            num_sweeps: 32,
            sweeps_per_beta: 4,
            beta_range: Some((0.1, 4.0)),
            seed: 29,
        };
        for variant in [
            DimerVariant::BaselineUnit,
            DimerVariant::PairUnit,
            DimerVariant::ReducedUnit,
            DimerVariant::ReducedPolishUnit,
        ] {
            let result = sample_clusters(&graph, &params, variant, &[vec![0, 1], vec![2, 3]])
                .expect("explicit matching");
            assert_eq!(result.spins.len(), 64);
        }
        for invalid in [
            vec![vec![0, 2]],
            vec![vec![0, 1], vec![1, 2]],
            vec![vec![0, 5]],
        ] {
            assert!(sample_clusters(&graph, &params, DimerVariant::PairUnit, &invalid).is_err());
        }
    }

    #[test]
    fn fractional_polish_keeps_exact_milli_incumbents_and_replays() {
        for weak in [0.031, 0.062] {
            let graph = IsingGraph::new(
                vec![0.0; 6],
                vec![-1.0, -1.0, weak, -weak, weak, -weak],
                vec![(0, 1), (2, 3), (0, 4), (1, 4), (2, 5), (3, 5)],
            );
            let params = SampleParams {
                num_reads: 64,
                num_sweeps: 32,
                sweeps_per_beta: 4,
                beta_range: Some((0.1, 0.2)),
                seed: 71,
            };
            let result = sample_dimer(&graph, &params, DimerVariant::ReducedPolishUnit)
                .expect("fractional polish");
            let replay = sample_dimer(&graph, &params, DimerVariant::ReducedPolishUnit)
                .expect("fractional replay");
            assert_eq!(result.spins, replay.spins);
            assert_eq!(
                result.solve_sweeps + result.polish_sweeps,
                params.num_sweeps
            );
            let energy = |spins: &[i8]| -> i64 {
                graph
                    .edges
                    .iter()
                    .zip(&graph.j)
                    .map(|(&(u, v), &j)| {
                        (j * 1000.0).round() as i64 * i64::from(spins[u]) * i64::from(spins[v])
                    })
                    .sum()
            };
            let expanded = result.expanded_spins.expect("expanded");
            let finals = result.final_spins.expect("finals");
            for ((chosen, incumbent), candidate) in result.spins.iter().zip(&expanded).zip(&finals)
            {
                assert_eq!(energy(chosen), energy(incumbent).min(energy(candidate)));
                if energy(incumbent) == energy(candidate) {
                    assert_eq!(chosen, incumbent);
                }
            }
        }
    }
}
