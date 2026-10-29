//! Exact conditional minima for signed strong paths.

use super::{spin, State, WeightedGraph, LANES};

struct Path {
    nodes: Vec<usize>,
    bonds: Vec<i32>,
}

pub(super) struct Paths {
    paths: Vec<Path>,
    boundary: Vec<[i32; LANES]>,
    predecessor: Vec<[u64; 2]>,
    words: Vec<u64>,
}

impl Paths {
    pub(super) fn new(graph: &WeightedGraph) -> Result<Self, String> {
        let mut strong = vec![Vec::new(); graph.neighbors.len()];
        for &(u, v, coupling) in &graph.edges {
            if coupling.abs() == 1000 {
                strong[u].push((v, coupling));
                strong[v].push((u, coupling));
            }
        }
        if strong.iter().any(|neighbors| neighbors.len() > 2) {
            return Err("strong path update rejects branches".to_owned());
        }
        let mut owner = vec![None; strong.len()];
        let mut paths = Vec::new();
        for start in 0..strong.len() {
            if strong[start].len() != 1 || owner[start].is_some() {
                continue;
            }
            let mut nodes = Vec::new();
            let mut bonds = Vec::new();
            let mut previous = None;
            let mut node = start;
            loop {
                if owner[node].is_some() {
                    return Err("strong path update encountered a repeated node".to_owned());
                }
                owner[node] = Some((paths.len(), nodes.len()));
                nodes.push(node);
                let following = strong[node]
                    .iter()
                    .find(|&&(next, _)| Some(next) != previous);
                let Some(&(next, coupling)) = following else {
                    break;
                };
                bonds.push(coupling);
                previous = Some(node);
                node = next;
            }
            paths.push(Path { nodes, bonds });
        }
        if strong
            .iter()
            .zip(&owner)
            .any(|(adjacent, owner)| !adjacent.is_empty() && owner.is_none())
        {
            return Err("strong path update rejects cycles".to_owned());
        }
        for &(u, v, coupling) in &graph.edges {
            if let (Some((path_u, position_u)), Some((path_v, position_v))) = (owner[u], owner[v]) {
                if path_u == path_v
                    && (coupling.abs() != 1000 || position_u.abs_diff(position_v) != 1)
                {
                    return Err("strong path update rejects internal chords".to_owned());
                }
            }
        }
        let Some(longest) = paths.iter().map(|path| path.nodes.len()).max() else {
            return Err("strong path update requires a signed 1000-milli bond".to_owned());
        };
        Ok(Self {
            paths,
            boundary: vec![[0; LANES]; longest],
            predecessor: vec![[0; 2]; longest],
            words: vec![0; longest],
        })
    }

    pub(super) fn minimize(&mut self, graph: &WeightedGraph, state: &mut State) {
        for path in &self.paths {
            let length = path.nodes.len();
            let mut incumbent = [0; LANES];
            for (position, &node) in path.nodes.iter().enumerate() {
                let fields = &mut self.boundary[position];
                *fields = state.fields[node];
                for lane in 0..LANES {
                    let value = spin(state.spins[node], lane);
                    if position > 0 {
                        let contribution = path.bonds[position - 1]
                            * spin(state.spins[path.nodes[position - 1]], lane);
                        fields[lane] -= contribution;
                        incumbent[lane] += value * contribution;
                    }
                    if position + 1 < length {
                        fields[lane] -= path.bonds[position]
                            * spin(state.spins[path.nodes[position + 1]], lane);
                    }
                    incumbent[lane] += value * fields[lane];
                }
            }
            let mut cost = [self.boundary[0], self.boundary[0].map(|field| -field)];
            for position in 1..length {
                let coupling = path.bonds[position - 1];
                self.predecessor[position] = [0; 2];
                let [plus_costs, minus_costs] = &mut cost;
                for (lane, (plus_cost, minus_cost)) in
                    plus_costs.iter_mut().zip(minus_costs).enumerate()
                {
                    let plus = *plus_cost;
                    let minus = *minus_cost;
                    let field = self.boundary[position][lane];
                    let plus_from_minus = minus - coupling < plus + coupling;
                    let minus_from_minus = minus + coupling < plus - coupling;
                    *plus_cost = field + (plus + coupling).min(minus - coupling);
                    *minus_cost = -field + (plus - coupling).min(minus + coupling);
                    self.predecessor[position][0] |= u64::from(plus_from_minus) << lane;
                    self.predecessor[position][1] |= u64::from(minus_from_minus) << lane;
                }
            }
            let mut improved = 0u64;
            let mut endpoint = 0u64;
            for (lane, &original) in incumbent.iter().enumerate() {
                improved |= u64::from(cost[0][lane].min(cost[1][lane]) < original) << lane;
                endpoint |= u64::from(cost[1][lane] < cost[0][lane]) << lane;
            }
            if improved == 0 {
                continue;
            }
            self.words[length - 1] = endpoint;
            for position in (1..length).rev() {
                let word = self.words[position];
                self.words[position - 1] = (self.predecessor[position][0] & !word)
                    | (self.predecessor[position][1] & word);
            }
            for (&node, &word) in path.nodes.iter().zip(&self.words) {
                state.apply(graph, node, (state.spins[node] ^ word) & improved);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dimer_weighted::tests::{assert_cache, lane};
    use crate::sa_msc::{MscState, LANES};
    use quip_solver_core::IsingGraph;
    use rand::rngs::SmallRng;
    use rand::SeedableRng;

    fn fixture(length: usize) -> IsingGraph {
        let mut edges = Vec::new();
        let mut weights = Vec::new();
        for node in 0..length {
            if node + 1 < length {
                edges.push((node, node + 1));
                weights.push(if node % 3 == 0 { 1.0 } else { -1.0 });
            }
            for outside in 0..2 {
                edges.push((node, length + 2 * node + outside));
                weights.push(if outside == 0 { 0.6 } else { -0.143 });
            }
        }
        IsingGraph::new(vec![0.0; length * 3], weights, edges)
    }

    #[test]
    fn signed_paths_match_exhaustive_conditional_minima_and_preserve_cache() {
        for length in [2, 3, 9, 11] {
            let base = fixture(length);
            let graph = WeightedGraph::new(&base).expect("integer graph");
            let mut paths = Paths::new(&graph).expect("simple strong path");
            let mut rng = SmallRng::seed_from_u64(733);
            let mut state = State::new(&graph, MscState::random(base.h.len(), &mut rng));
            let before: Vec<_> = (0..LANES).map(|index| lane(&state, index)).collect();
            paths.minimize(&graph, &mut state);
            let mut improvements = 0;
            for (index, original) in before.iter().enumerate() {
                let optimum = (0..1usize << length)
                    .map(|bits| {
                        let mut trial = original.clone();
                        for (node, value) in trial[..length].iter_mut().enumerate() {
                            *value = if bits & (1 << node) == 0 { 1 } else { -1 };
                        }
                        graph.energy(&trial)
                    })
                    .min()
                    .expect("nonempty assignments");
                let result = lane(&state, index);
                assert_eq!(
                    graph.energy(&result),
                    optimum,
                    "length {length}, lane {index}"
                );
                assert_eq!(result[length..], original[length..]);
                improvements += usize::from(optimum < graph.energy(original));
            }
            assert!(improvements > 0);
            assert_cache(&graph, &state);
            let optimum_words = state.spins.clone();
            paths.minimize(&graph, &mut state);
            assert_eq!(state.spins, optimum_words, "tied incumbents must remain");
            assert_cache(&graph, &state);
        }
    }

    #[test]
    fn public_path_sampler_handles_more_than_one_packed_word_and_replays() {
        use crate::dimer_research::{sample_clusters, DimerVariant};
        use quip_solver_core::SampleParams;

        let edges = (0..8).map(|node| (node, node + 1)).collect();
        let weights = (0..8)
            .map(|node| if node % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let base = IsingGraph::new(vec![0.0; 9], weights, edges);
        let params = SampleParams {
            num_reads: 65,
            num_sweeps: 16,
            sweeps_per_beta: 1,
            beta_range: Some((0.01, 2.5)),
            seed: 94,
        };
        let result = sample_clusters(&base, &params, DimerVariant::PathBlockWeighted, &[])
            .expect("path sampler");
        let replay = sample_clusters(&base, &params, DimerVariant::PathBlockWeighted, &[])
            .expect("repeat path sampler");
        assert_eq!(result.spins, replay.spins);
        assert_eq!(result.spins.len(), 65);
        let graph = WeightedGraph::new(&base).expect("integer graph");
        assert!(result
            .spins
            .iter()
            .all(|spins| graph.energy(spins) == -8000));
    }

    #[test]
    fn strong_branches_cycles_and_internal_weak_chords_are_rejected() {
        let cases = [
            (vec![(0, 1), (1, 2), (1, 3)], vec![-1.0; 3]),
            (vec![(0, 1), (1, 2), (2, 3), (0, 3)], vec![-1.0; 4]),
            (
                vec![(0, 1), (1, 2), (2, 3), (0, 3)],
                vec![-1.0, 1.0, -1.0, 0.2],
            ),
        ];
        for (edges, weights) in cases {
            let graph = WeightedGraph::new(&IsingGraph::new(vec![0.0; 4], weights, edges))
                .expect("integer graph");
            assert!(Paths::new(&graph).is_err());
        }
    }
}
