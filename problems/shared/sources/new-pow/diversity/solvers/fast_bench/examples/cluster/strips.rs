use rand::rngs::SmallRng;
use rand::Rng;

const STRIP_COUNT: usize = 200;
const LAYER_COUNT: usize = 12;
const STATE_COUNT: usize = 4;
const MISSING_NODE: usize = usize::MAX;

#[derive(Clone, Copy)]
struct Coordinate {
    group: usize,
    layer: usize,
    rail: usize,
    orientation: usize,
}

#[derive(Clone, Copy)]
struct BoundaryEdge {
    layer: usize,
    rail: usize,
    other: usize,
    weight: i32,
}

struct Strip {
    nodes: [[usize; 2]; LAYER_COUNT],
    present: [u8; LAYER_COUNT],
    within: [[i64; STATE_COUNT]; LAYER_COUNT],
    transition: [[[i64; STATE_COUNT]; STATE_COUNT]; LAYER_COUNT - 1],
    boundary: Vec<BoundaryEdge>,
}

impl Strip {
    fn empty() -> Self {
        Self {
            nodes: [[MISSING_NODE; 2]; LAYER_COUNT],
            present: [0; LAYER_COUNT],
            within: [[0; STATE_COUNT]; LAYER_COUNT],
            transition: [[[0; STATE_COUNT]; STATE_COUNT]; LAYER_COUNT - 1],
            boundary: Vec::new(),
        }
    }
}

pub(super) struct StripTopology {
    n: usize,
    strips: Vec<Strip>,
}

impl StripTopology {
    pub(super) fn new(
        n: usize,
        physical_nodes: &[usize],
        edges: &[(usize, usize, i32)],
    ) -> Result<Self, String> {
        if physical_nodes.len() != n {
            return Err("physical node count does not match spin count".into());
        }

        let mut physical_seen = [false; STRIP_COUNT * 24];
        let mut coordinates = Vec::with_capacity(n);
        let mut strips: Vec<Strip> = (0..STRIP_COUNT).map(|_| Strip::empty()).collect();
        for (node, &physical) in physical_nodes.iter().enumerate() {
            if physical >= physical_seen.len() {
                return Err("physical node is outside Z(12,4)".into());
            }
            if physical_seen[physical] {
                return Err("physical nodes must be unique".into());
            }
            physical_seen[physical] = true;

            let coordinate = Coordinate {
                group: physical / 24,
                layer: physical % LAYER_COUNT,
                rail: (physical / LAYER_COUNT) % 2,
                orientation: (physical / 24) / 100,
            };
            strips[coordinate.group].nodes[coordinate.layer][coordinate.rail] = node;
            strips[coordinate.group].present[coordinate.layer] |= 1 << coordinate.rail;
            coordinates.push(coordinate);
        }

        for &(u, v, weight) in edges {
            if u >= n || v >= n || u == v {
                return Err("edge has an invalid endpoint".into());
            }
            let a = coordinates[u];
            let b = coordinates[v];
            if a.orientation == b.orientation {
                if a.group != b.group {
                    return Err("same-orientation edge crosses strip groups".into());
                }
                let layer_distance = a.layer.abs_diff(b.layer);
                if layer_distance > 1 || (layer_distance == 0 && a.rail == b.rail) {
                    return Err("same-orientation edge violates strip geometry".into());
                }
                Self::add_internal_edge(&mut strips[a.group], a, b, weight);
            } else {
                strips[a.group].boundary.push(BoundaryEdge {
                    layer: a.layer,
                    rail: a.rail,
                    other: v,
                    weight,
                });
                strips[b.group].boundary.push(BoundaryEdge {
                    layer: b.layer,
                    rail: b.rail,
                    other: u,
                    weight,
                });
            }
        }

        strips.retain(|strip| strip.present.iter().any(|&present| present != 0));

        Ok(Self { n, strips })
    }

    fn add_internal_edge(strip: &mut Strip, a: Coordinate, b: Coordinate, weight: i32) {
        if a.layer == b.layer {
            for state in 0..STATE_COUNT {
                strip.within[a.layer][state] += i64::from(weight)
                    * i64::from(spin_for_state(state, a.rail))
                    * i64::from(spin_for_state(state, b.rail));
            }
            return;
        }

        let (lower, upper) = if a.layer < b.layer { (a, b) } else { (b, a) };
        for lower_state in 0..STATE_COUNT {
            for upper_state in 0..STATE_COUNT {
                strip.transition[lower.layer][lower_state][upper_state] += i64::from(weight)
                    * i64::from(spin_for_state(lower_state, lower.rail))
                    * i64::from(spin_for_state(upper_state, upper.rail));
            }
        }
    }

    pub(super) fn sweep(&self, spins: &mut [i8], beta: f64, rng: &mut SmallRng) -> i64 {
        debug_assert_eq!(spins.len(), self.n);
        debug_assert!(beta.is_finite() && beta >= 0.0 || beta == f64::INFINITY);

        let mut delta = 0_i64;
        for strip in &self.strips {
            delta += update_strip(strip, spins, beta, rng);
        }
        delta
    }
}

fn update_strip(strip: &Strip, spins: &mut [i8], beta: f64, rng: &mut SmallRng) -> i64 {
    let mut fields = [[0_i64; 2]; LAYER_COUNT];
    for edge in &strip.boundary {
        fields[edge.layer][edge.rail] += i64::from(edge.weight) * i64::from(spins[edge.other]);
    }

    let mut local = [[0_i64; STATE_COUNT]; LAYER_COUNT];
    for (layer, local_layer) in local.iter_mut().enumerate() {
        for (state, energy) in local_layer.iter_mut().enumerate() {
            *energy = strip.within[layer][state]
                + fields[layer][0] * i64::from(spin_for_state(state, 0))
                + fields[layer][1] * i64::from(spin_for_state(state, 1));
        }
    }

    let old_states = states_from_spins(strip, spins);
    let old_energy = strip_energy(strip, &local, &old_states);
    let new_states = if beta == f64::INFINITY {
        minimum_states(strip, &local, rng)
    } else {
        gibbs_states(strip, &local, beta, rng)
    };
    let new_energy = strip_energy(strip, &local, &new_states);

    for (layer, &new_state) in new_states.iter().enumerate() {
        for rail in 0..2 {
            let node = strip.nodes[layer][rail];
            if node != MISSING_NODE {
                spins[node] = spin_for_state(new_state, rail);
            }
        }
    }

    new_energy - old_energy
}

fn states_from_spins(strip: &Strip, spins: &[i8]) -> [usize; LAYER_COUNT] {
    let mut states = [0; LAYER_COUNT];
    for (layer, state) in states.iter_mut().enumerate() {
        for rail in 0..2 {
            let node = strip.nodes[layer][rail];
            if node != MISSING_NODE && spins[node] > 0 {
                *state |= 1 << rail;
            }
        }
    }
    states
}

fn strip_energy(
    strip: &Strip,
    local: &[[i64; STATE_COUNT]; LAYER_COUNT],
    states: &[usize; LAYER_COUNT],
) -> i64 {
    let mut energy = local[0][states[0]];
    for layer in 1..LAYER_COUNT {
        energy += local[layer][states[layer]]
            + strip.transition[layer - 1][states[layer - 1]][states[layer]];
    }
    energy
}

fn minimum_states(
    strip: &Strip,
    local: &[[i64; STATE_COUNT]; LAYER_COUNT],
    rng: &mut SmallRng,
) -> [usize; LAYER_COUNT] {
    let mut costs = [[i64::MAX; STATE_COUNT]; LAYER_COUNT];
    for state in valid_states(strip.present[0]) {
        costs[0][state] = local[0][state];
    }
    for layer in 1..LAYER_COUNT {
        for state in valid_states(strip.present[layer]) {
            let mut best = i64::MAX;
            for previous in valid_states(strip.present[layer - 1]) {
                best = best
                    .min(costs[layer - 1][previous] + strip.transition[layer - 1][previous][state]);
            }
            costs[layer][state] = local[layer][state] + best;
        }
    }

    let mut states = [0; LAYER_COUNT];
    states[LAYER_COUNT - 1] = choose_minimum(
        |state| costs[LAYER_COUNT - 1][state],
        strip.present[LAYER_COUNT - 1],
        rng,
    );
    for layer in (0..LAYER_COUNT - 1).rev() {
        let next = states[layer + 1];
        states[layer] = choose_minimum(
            |state| costs[layer][state] + strip.transition[layer][state][next],
            strip.present[layer],
            rng,
        );
    }
    states
}

fn choose_minimum(mut cost: impl FnMut(usize) -> i64, present: u8, rng: &mut SmallRng) -> usize {
    let mut chosen = 0;
    let mut best = i64::MAX;
    let mut ties = 0_u32;
    for state in valid_states(present) {
        let candidate = cost(state);
        if candidate < best {
            best = candidate;
            chosen = state;
            ties = 1;
        } else if candidate == best {
            ties += 1;
            if rng.gen_range(0..ties) == 0 {
                chosen = state;
            }
        }
    }
    chosen
}

fn gibbs_states(
    strip: &Strip,
    local: &[[i64; STATE_COUNT]; LAYER_COUNT],
    beta: f64,
    rng: &mut SmallRng,
) -> [usize; LAYER_COUNT] {
    let scale = -beta / 1000.0;
    let mut forward = [[f64::NEG_INFINITY; STATE_COUNT]; LAYER_COUNT];
    for state in valid_states(strip.present[0]) {
        forward[0][state] = scale * local[0][state] as f64;
    }
    for layer in 1..LAYER_COUNT {
        for state in valid_states(strip.present[layer]) {
            let mut weights = [f64::NEG_INFINITY; STATE_COUNT];
            for previous in valid_states(strip.present[layer - 1]) {
                weights[previous] = forward[layer - 1][previous]
                    + scale * strip.transition[layer - 1][previous][state] as f64;
            }
            forward[layer][state] = scale * local[layer][state] as f64
                + log_sum_exp(&weights, strip.present[layer - 1]);
        }
    }

    let mut states = [0; LAYER_COUNT];
    states[LAYER_COUNT - 1] = sample_log_weights(
        &forward[LAYER_COUNT - 1],
        strip.present[LAYER_COUNT - 1],
        rng,
    );
    for layer in (0..LAYER_COUNT - 1).rev() {
        let next = states[layer + 1];
        let mut weights = [f64::NEG_INFINITY; STATE_COUNT];
        for state in valid_states(strip.present[layer]) {
            weights[state] =
                forward[layer][state] + scale * strip.transition[layer][state][next] as f64;
        }
        states[layer] = sample_log_weights(&weights, strip.present[layer], rng);
    }
    states
}

fn log_sum_exp(weights: &[f64; STATE_COUNT], present: u8) -> f64 {
    let mut maximum = f64::NEG_INFINITY;
    for state in valid_states(present) {
        maximum = maximum.max(weights[state]);
    }
    let mut total = 0.0;
    for state in valid_states(present) {
        total += (weights[state] - maximum).exp();
    }
    maximum + total.ln()
}

fn sample_log_weights(weights: &[f64; STATE_COUNT], present: u8, rng: &mut SmallRng) -> usize {
    let mut maximum = f64::NEG_INFINITY;
    for state in valid_states(present) {
        maximum = maximum.max(weights[state]);
    }
    let mut total = 0.0;
    for state in valid_states(present) {
        total += (weights[state] - maximum).exp();
    }

    let target = rng.gen::<f64>() * total;
    let mut cumulative = 0.0;
    let mut fallback = 0;
    for state in valid_states(present) {
        fallback = state;
        cumulative += (weights[state] - maximum).exp();
        if target < cumulative {
            return state;
        }
    }
    fallback
}

fn valid_states(present: u8) -> impl Iterator<Item = usize> {
    (0..STATE_COUNT).filter(move |&state| state as u8 & !present == 0)
}

fn spin_for_state(state: usize, rail: usize) -> i8 {
    if state & (1 << rail) == 0 {
        -1
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::StripTopology;
    use rand::rngs::SmallRng;
    use rand::SeedableRng;

    fn energy(spins: &[i8], edges: &[(usize, usize, i32)]) -> i64 {
        edges
            .iter()
            .map(|&(u, v, weight)| i64::from(weight) * i64::from(spins[u]) * i64::from(spins[v]))
            .sum()
    }

    #[test]
    fn infinite_beta_finds_strip_minimum_with_boundary_fields() {
        let physical_nodes = [0, 4776, 4788, 4777, 4789, 4779, 4791];
        let edges = [
            (1, 2, 700),
            (1, 3, -300),
            (2, 4, 400),
            (3, 4, -900),
            (5, 6, 500),
            (0, 1, 600),
            (0, 4, -250),
        ];
        let topology = StripTopology::new(physical_nodes.len(), &physical_nodes, &edges)
            .expect("valid damaged strip topology");
        let mut spins = [1, 1, -1, 1, 1, -1, -1];
        let mut rng = SmallRng::seed_from_u64(7);

        topology.sweep(&mut spins, f64::INFINITY, &mut rng);

        let actual = energy(&spins, &edges);
        let boundary_spin = spins[0];
        let mut best = i64::MAX;
        for bits in 0_u8..64 {
            let mut candidate = [boundary_spin; 7];
            for node in 1..7 {
                candidate[node] = if bits & (1 << (node - 1)) == 0 { -1 } else { 1 };
            }
            best = best.min(energy(&candidate, &edges));
        }
        assert_eq!(actual, best);
    }

    #[test]
    fn finite_beta_two_spin_energy_matches_boltzmann_mean() {
        let physical_nodes = [0, 12];
        let edges = [(0, 1, 1000)];
        let topology =
            StripTopology::new(2, &physical_nodes, &edges).expect("valid two-spin strip topology");
        let mut spins = [1, 1];
        let mut rng = SmallRng::seed_from_u64(19);
        let beta = 0.7;
        let samples = 30_000;
        let mut total = 0_i64;

        for _ in 0..samples {
            topology.sweep(&mut spins, beta, &mut rng);
            total += energy(&spins, &edges);
        }

        let empirical = total as f64 / f64::from(samples);
        let expected = -1000.0 * beta.tanh();
        assert!(
            (empirical - expected).abs() < 25.0,
            "{empirical} != {expected}"
        );
    }

    #[test]
    fn sweep_delta_matches_full_edge_rescore() {
        let physical_nodes = [0, 12, 1, 2400, 2412, 2401];
        let edges = [
            (0, 1, 1000),
            (0, 2, -125),
            (1, 2, 62),
            (3, 4, -1000),
            (3, 5, 125),
            (0, 3, -62),
            (1, 4, 1000),
            (2, 5, -125),
        ];
        let topology = StripTopology::new(physical_nodes.len(), &physical_nodes, &edges)
            .expect("valid two-orientation topology");
        let mut spins = [1, -1, 1, -1, -1, 1];
        let before = energy(&spins, &edges);
        let mut rng = SmallRng::seed_from_u64(29);

        let delta = topology.sweep(&mut spins, 1.3, &mut rng);

        assert_eq!(delta, energy(&spins, &edges) - before);
    }

    #[test]
    fn missing_layers_and_rails_are_valid() {
        let physical_nodes = [0, 13, 3, 16, 7];
        let edges = [(0, 1, 250), (2, 3, -700)];
        let topology = StripTopology::new(physical_nodes.len(), &physical_nodes, &edges)
            .expect("missing qubits must remain valid");
        let mut spins = [1, 1, -1, 1, -1];
        let before = energy(&spins, &edges);
        let mut rng = SmallRng::seed_from_u64(37);

        let delta = topology.sweep(&mut spins, f64::INFINITY, &mut rng);

        assert_eq!(delta, energy(&spins, &edges) - before);
        assert_eq!(energy(&spins, &edges), -950);
    }

    #[test]
    fn constructor_rejects_nonlocal_same_orientation_edges() {
        let physical_nodes = [0, 24, 3];
        assert!(StripTopology::new(3, &physical_nodes, &[(0, 1, 1)]).is_err());
        assert!(StripTopology::new(3, &physical_nodes, &[(0, 2, 1)]).is_err());
    }
}
