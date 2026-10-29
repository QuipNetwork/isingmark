//! Pair-aware auxiliary-bond optimization for round 2.
//! Pair parity can be fixed or updated separately between unit-cluster moves.
//! The local graph weights follow Otsuka, Phys. Rev. B 90, 220406 (2014).

use clap::{Parser, ValueEnum};
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::time::Instant;

#[path = "pair/exact.rs"]
mod exact;
#[path = "cluster/strips.rs"]
mod strips;
#[path = "pair/tabu.rs"]
mod tabu;

#[derive(Clone, Copy, Debug, ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
enum Schedule {
    Pt,
    Anneal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
enum PairMode {
    Soft,
    Fixed,
    Dynamic,
}

fn contraction(n: usize, pairs: &[[usize; 2]]) -> Result<Vec<usize>, String> {
    let mut partner: Vec<usize> = (0..n).collect();
    for &[u, v] in pairs {
        if u >= n || v >= n || u == v || partner[u] != u || partner[v] != v {
            return Err("invalid or overlapping pair".into());
        }
        partner[u] = v;
        partner[v] = u;
    }
    let mut result = vec![usize::MAX; n];
    let mut next = 0;
    for u in 0..n {
        if result[u] == usize::MAX {
            result[u] = next;
            result[partner[u]] = next;
            next += 1;
        }
    }
    Ok(result)
}

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    sw_aggregate: bool,
    #[arg(long, default_value_t = 0.0)]
    cpu_limit_s: f64,
    #[arg(long, default_value_t = 0)]
    tabu_steps: usize,
    #[arg(long, default_value_t = 0)]
    exact_ball: usize,
    #[arg(long, default_value_t = false, action = clap::ArgAction::Set)]
    warm_restarts: bool,
    #[arg(long, default_value_t = 1.5)]
    reheat_beta: f64,
    #[arg(long, default_value_t = 0)]
    exact_passes: usize,
    #[arg(long, default_value_t = 0)]
    exact_every: usize,
    #[arg(long, default_value_t = 2)]
    exact_strips: usize,
    #[arg(long, default_value_t = 12)]
    exact_width: usize,
    #[arg(long, value_enum, default_value_t = PairMode::Dynamic)]
    pair_mode: PairMode,
    #[arg(long, default_value_t = 1000)]
    tile_weight_milli: i32,
    #[arg(long, default_value_t = 0.0)]
    sw_fraction: f64,
    #[arg(long, default_value_t = 1)]
    sw_every: usize,
    #[arg(long, default_value_t = 0)]
    houdayer_every: usize,
    #[arg(long, default_value_t = 1)]
    copies: usize,
    #[arg(long)]
    jobs: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, default_value_t = 4096)]
    sweeps: usize,
    #[arg(long, default_value_t = 16)]
    replicas: usize,
    #[arg(long, default_value_t = 0.5)]
    beta_min: f64,
    #[arg(long, default_value_t = 16.0)]
    beta_max: f64,
    #[arg(long, default_value_t = 0)]
    single_every: usize,
    #[arg(long, default_value_t = 0)]
    strip_every: usize,
    #[arg(long, default_value_t = 1)]
    cluster_every: usize,
    #[arg(long, default_value_t = 0)]
    polish_sweeps: usize,
    #[arg(long, default_value_t = 1000)]
    dimer_weight_milli: i32,
    #[arg(long, default_value_t = 0)]
    seed_offset: u64,
    #[arg(long, value_enum, default_value_t = Schedule::Pt)]
    schedule: Schedule,
    #[arg(long, default_value_t = 1)]
    restarts: usize,
    /// Hex-packed initial state (encode_spins format). Every replica starts from it and the
    /// first anneal starts at reheat_beta instead of beta_min.
    #[arg(long)]
    init_hex: Option<String>,
}

fn decode_spins(hex: &str, n: usize) -> Result<Vec<i8>, String> {
    let bytes: Result<Vec<u8>, _> = (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16))
        .collect();
    let bytes = bytes.map_err(|e| e.to_string())?;
    if bytes.len() != n.div_ceil(8) {
        return Err("init_hex length does not match the model".into());
    }
    Ok((0..n)
        .map(|i| if bytes[i / 8] & (1 << (i % 8)) != 0 { -1 } else { 1 })
        .collect())
}

#[derive(Deserialize)]
struct Job {
    nonce: String,
    model_hash: String,
    seed: u64,
    h: Vec<f64>,
    edges: Vec<[usize; 2]>,
    j: Vec<f64>,
    clusters: Vec<[usize; 4]>,
    #[serde(default)]
    pairs: Vec<[usize; 2]>,
    #[serde(default)]
    physical_nodes: Vec<usize>,
}

struct Tile {
    nodes: [usize; 4],
    gauge: [i8; 4],
}

struct Graph {
    tile_weight_milli: i32,
    pair_mode: PairMode,
    sw_fraction: f64,
    sw_aggregate: bool,
    units: Vec<Vec<usize>>,
    unit_of: Vec<usize>,
    unit_neighbors: Vec<Vec<usize>>,
    weak_units: Vec<Vec<(usize, usize, i32)>>,
    n: usize,
    tiles: Vec<Tile>,
    pairs: Vec<[usize; 2]>,
    dimer_weight_milli: i32,
    weak: Vec<Vec<(usize, i32)>>,
    all: Vec<Vec<(usize, i32)>>,
    edges: Vec<(usize, usize, i32)>,
    /// Compact CSR copies of `weak` and `all` for the hot loops (4 bytes per entry; the
    /// chip has fewer than 65,536 spins and every coupling fits in i16 milli).
    weak_off: Vec<u32>,
    weak_adj: Vec<(u16, i16)>,
    all_off: Vec<u32>,
    all_adj: Vec<(u16, i16)>,
}

fn csr(lists: &[Vec<(usize, i32)>]) -> (Vec<u32>, Vec<(u16, i16)>) {
    let mut off = Vec::with_capacity(lists.len() + 1);
    let mut adj = Vec::new();
    off.push(0);
    for list in lists {
        adj.extend(list.iter().map(|&(v, j)| (v as u16, j as i16)));
        off.push(adj.len() as u32);
    }
    (off, adj)
}

impl Graph {
    fn new(job: &Job, dimer_weight_milli: i32) -> Result<Self, String> {
        if !(0..=1000).contains(&dimer_weight_milli) {
            return Err("dimer weight must be in 0..=1000 milli".into());
        }
        let n = job.h.len();
        if n > usize::from(u16::MAX) {
            return Err("compact adjacency supports at most 65,535 spins".into());
        }
        if n == 0 || job.h.iter().any(|&h| h != 0.0) {
            return Err("cluster solver requires nonempty zero-field graph".into());
        }
        if job.edges.len() != job.j.len() {
            return Err("edge/coefficient length mismatch".into());
        }
        let mut weights = HashMap::new();
        let mut edges = Vec::new();
        for (&[u, v], &j) in job.edges.iter().zip(&job.j) {
            let j_milli = (j * 1000.0).round();
            if u >= n
                || v >= n
                || u == v
                || !j.is_finite()
                || j_milli.abs() > 1000.0
                || (j_milli - j * 1000.0).abs() > 1e-8
            {
                return Err("invalid edge or noninteger milliweight".into());
            }
            let key = (u.min(v), u.max(v));
            if weights.insert(key, j_milli as i32).is_some() {
                return Err("duplicate edge".into());
            }
            edges.push((u, v, j_milli as i32));
        }
        let mut strong = HashSet::new();
        let mut tiles = Vec::new();
        for &nodes in &job.clusters {
            let mut gauge = [1_i8; 4];
            for k in 1..4 {
                let key = (nodes[0].min(nodes[k]), nodes[0].max(nodes[k]));
                gauge[k] = weights
                    .get(&key)
                    .ok_or("missing tetrahedron edge")?
                    .signum() as i8;
            }
            for a in 0..4 {
                for b in a + 1..4 {
                    let key = (nodes[a].min(nodes[b]), nodes[a].max(nodes[b]));
                    if weights.get(&key) != Some(&(1000 * i32::from(gauge[a] * gauge[b])))
                        || !strong.insert(key)
                    {
                        return Err("tetrahedra must be edge-disjoint gauged 1000-milli K4s".into());
                    }
                }
            }
            tiles.push(Tile { nodes, gauge });
        }
        let mut paired = vec![false; n];
        for &[u, v] in &job.pairs {
            if u >= n || v >= n || paired[u] || paired[v] {
                return Err("dimers must have valid disjoint endpoints".into());
            }
            let key = (u.min(v), u.max(v));
            if weights.get(&key) != Some(&-1000) || !strong.insert(key) {
                return Err("dimer must be an extra -1000-milli edge".into());
            }
            paired[u] = true;
            paired[v] = true;
        }
        let mut weak = vec![Vec::new(); n];
        let mut all = vec![Vec::new(); n];
        for &(u, v, j) in &edges {
            all[u].push((v, j));
            all[v].push((u, j));
            if !strong.contains(&(u.min(v), u.max(v))) {
                if j.abs() >= 1000 {
                    return Err("unclassified strong edge".into());
                }
                weak[u].push((v, j));
                weak[v].push((u, j));
            }
        }
        let residual_dimer_weight = dimer_weight_milli - 1000;
        if residual_dimer_weight != 0 {
            for &[u, v] in &job.pairs {
                weak[u].push((v, residual_dimer_weight));
                weak[v].push((u, residual_dimer_weight));
            }
        }
        let unit_of = contraction(n, &job.pairs)?;
        let mut units = vec![Vec::new(); unit_of.iter().max().copied().unwrap_or(0) + 1];
        for (u, &unit) in unit_of.iter().enumerate() {
            units[unit].push(u);
        }
        let (weak_off, weak_adj) = csr(&weak);
        let (all_off, all_adj) = csr(&all);
        Ok(Self {
            tile_weight_milli: 1000,
            pair_mode: PairMode::Soft,
            sw_fraction: 0.0,
            sw_aggregate: true,
            units,
            unit_of,
            unit_neighbors: Vec::new(),
            weak_units: Vec::new(),
            n,
            tiles,
            pairs: job.pairs.clone(),
            dimer_weight_milli,
            weak,
            all,
            edges,
            weak_off,
            weak_adj,
            all_off,
            all_adj,
        })
    }

    fn configure(&mut self, tile_weight: i32, pair_mode: PairMode, sw_fraction: f64) {
        self.tile_weight_milli = tile_weight;
        self.pair_mode = pair_mode;
        self.sw_fraction = sw_fraction;
        for t in &self.tiles {
            for a in 0..4 {
                for b in a + 1..4 {
                    let j = (1000 - tile_weight) * i32::from(t.gauge[a] * t.gauge[b]);
                    if j != 0 {
                        self.weak[t.nodes[a]].push((t.nodes[b], j));
                        self.weak[t.nodes[b]].push((t.nodes[a], j));
                    }
                }
            }
        }
        let mut groups = BTreeMap::<(usize, usize), Vec<(usize, usize, i32)>>::new();
        for (u, neighbors) in self.weak.iter().enumerate() {
            for &(v, j) in neighbors {
                if u >= v {
                    continue;
                }
                let (a, b) = if pair_mode == PairMode::Soft {
                    (u, v)
                } else {
                    (self.unit_of[u], self.unit_of[v])
                };
                if a != b {
                    groups
                        .entry((a.min(b), a.max(b)))
                        .or_default()
                        .push((u, v, j));
                }
            }
        }
        self.weak_units = groups.into_values().collect();
        (self.weak_off, self.weak_adj) = csr(&self.weak);
        (self.all_off, self.all_adj) = csr(&self.all);
    }

    fn energy(&self, s: &[i8]) -> i64 {
        self.edges
            .iter()
            .map(|&(u, v, j)| i64::from(j) * i64::from(s[u]) * i64::from(s[v]))
            .sum()
    }

    fn strong_energy(&self, s: &[i8]) -> i64 {
        let mut e = 0;
        for t in &self.tiles {
            let q: i64 = (0..4).map(|k| i64::from(t.gauge[k] * s[t.nodes[k]])).sum();
            e += i64::from(self.tile_weight_milli) * (q * q - 4) / 2;
        }
        for &[u, v] in &self.pairs {
            e -= i64::from(self.dimer_weight_milli) * i64::from(s[u]) * i64::from(s[v]);
        }
        e
    }
}

#[derive(Clone)]
struct State {
    spins: Vec<i8>,
    weak_energy: i64,
    energy: i64,
}

impl State {
    fn random(graph: &Graph, rng: &mut SmallRng) -> Self {
        let mut spins: Vec<i8> = (0..graph.n)
            .map(|_| if rng.gen::<bool>() { 1 } else { -1 })
            .collect();
        if graph.pair_mode != PairMode::Soft {
            for &[u, v] in &graph.pairs {
                spins[v] = spins[u];
            }
        }
        let energy = graph.energy(&spins);
        let weak_energy = energy - graph.strong_energy(&spins);
        Self {
            spins,
            weak_energy,
            energy,
        }
    }
}

fn graph_weights(beta: f64) -> [f64; 3] {
    let z = (-2.0 * beta).exp();
    let w0 = z.powi(4);
    [w0, (z - w0) / 3.0, (3.0 - 4.0 * z + w0) / 6.0]
}

struct Breakup {
    zero_ice: f64,
    single_ice: f64,
    zero_defect: f64,
    dimer_link: f64,
}

impl Breakup {
    fn for_graph(beta: f64, graph: &Graph) -> Self {
        let mut p = Self::new(
            beta * f64::from(graph.tile_weight_milli) / 1000.0,
            graph.dimer_weight_milli,
        );
        p.dimer_link = -(-2.0 * beta * f64::from(graph.dimer_weight_milli) / 1000.0).exp_m1();
        p
    }

    fn new(beta: f64, dimer_weight_milli: i32) -> Self {
        let [w0, w1, _] = graph_weights(beta);
        let dimer_link = if dimer_weight_milli == 1000 {
            -(-2.0 * beta).exp_m1()
        } else {
            let coupling = f64::from(dimer_weight_milli) / 1000.0;
            -(-2.0 * beta * coupling).exp_m1()
        };
        Self {
            zero_ice: w0,
            single_ice: w0 + 4.0 * w1,
            zero_defect: (-6.0 * beta).exp(),
            dimer_link,
        }
    }
}

struct Workspace {
    component_edges: Vec<(u64, i64)>,
    parent: Vec<usize>,
    size: Vec<usize>,
    label: Vec<usize>,
    head: Vec<usize>,
    next: Vec<usize>,
    /// Per-root boundary sum: sum of j * s_u * s_v over weak edges that leave the component.
    acc: Vec<i64>,
}

impl Workspace {
    fn new(n: usize) -> Self {
        Self {
            component_edges: Vec::new(),
            parent: vec![0; n],
            size: vec![1; n],
            label: vec![0; n],
            head: vec![usize::MAX; n],
            next: vec![usize::MAX; n],
            acc: vec![0; n],
        }
    }

    /// One sequential pass over the compact weak adjacency fills `acc` for every component.
    fn boundary_sums(&mut self, graph: &Graph, s: &[i8]) {
        self.acc.fill(0);
        for u in 0..graph.n {
            let lu = self.label[u];
            let mut a = 0_i64;
            for &(v, j) in &graph.weak_adj[graph.weak_off[u] as usize..graph.weak_off[u + 1] as usize] {
                if self.label[v as usize] != lu {
                    a += i64::from(j) * i64::from(s[v as usize]);
                }
            }
            self.acc[lu] += i64::from(s[u]) * a;
        }
    }

    fn find(&mut self, mut u: usize) -> usize {
        while self.parent[u] != u {
            self.parent[u] = self.parent[self.parent[u]];
            u = self.parent[u];
        }
        u
    }

    fn join(&mut self, a: usize, b: usize) {
        let mut a = self.find(a);
        let mut b = self.find(b);
        if a == b {
            return;
        }
        if self.size[a] < self.size[b] {
            std::mem::swap(&mut a, &mut b);
        }
        self.parent[b] = a;
        self.size[a] += self.size[b];
    }

    fn partition(&mut self, graph: &Graph, s: &[i8], probabilities: &Breakup, rng: &mut SmallRng) {
        for i in 0..graph.n {
            self.parent[i] = i;
            self.size[i] = 1;
        }
        if graph.pair_mode != PairMode::Soft {
            for &[u, v] in &graph.pairs {
                self.join(u, v);
            }
        }
        for tile in &graph.tiles {
            let mut positive = [0; 4];
            let mut negative = [0; 4];
            let mut np = 0;
            let mut nn = 0;
            for k in 0..4 {
                if s[tile.nodes[k]] * tile.gauge[k] > 0 {
                    positive[np] = tile.nodes[k];
                    np += 1;
                } else {
                    negative[nn] = tile.nodes[k];
                    nn += 1;
                }
            }
            if np == 0 || np == 4 {
                continue;
            }
            let r = rng.gen::<f64>();
            if np == 2 {
                if r < probabilities.zero_ice {
                    continue;
                }
                if r < probabilities.single_ice {
                    self.join(positive[rng.gen_range(0..2)], negative[rng.gen_range(0..2)]);
                } else {
                    let choice = usize::from(rng.gen::<bool>());
                    self.join(positive[0], negative[choice]);
                    self.join(positive[1], negative[1 - choice]);
                }
            } else if r >= probabilities.zero_defect {
                self.join(
                    positive[rng.gen_range(0..np)],
                    negative[rng.gen_range(0..nn)],
                );
            }
        }
        for &[u, v] in &graph.pairs {
            if graph.pair_mode == PairMode::Soft
                && s[u] == s[v]
                && rng.gen::<f64>() < probabilities.dimer_link
            {
                self.join(u, v);
            }
        }
        self.label_components(graph.n);
    }

    fn label_components(&mut self, n: usize) {
        self.head.fill(usize::MAX);
        for i in 0..n {
            let root = self.find(i);
            self.label[i] = root;
            self.next[i] = self.head[root];
            self.head[root] = i;
        }
    }

    #[cfg(test)]
    fn weak_delta(&self, graph: &Graph, s: &[i8], root: usize) -> i64 {
        let mut delta = 0_i64;
        let mut u = self.head[root];
        while u != usize::MAX {
            let su = i64::from(s[u]);
            let mut acc = 0_i64;
            for &(v, j) in &graph.weak_adj[graph.weak_off[u] as usize..graph.weak_off[u + 1] as usize] {
                if self.label[v as usize] != root {
                    acc += i64::from(j) * i64::from(s[v as usize]);
                }
            }
            delta -= 2 * su * acc;
            u = self.next[u];
        }
        delta
    }

    fn sweep(
        &mut self,
        graph: &Graph,
        state: &mut State,
        beta: f64,
        probabilities: &Breakup,
        rng: &mut SmallRng,
        stats: &mut Stats,
    ) {
        let t0 = Instant::now();
        self.partition(graph, &state.spins, probabilities, rng);
        stats.t_partition_ns += t0.elapsed().as_nanos() as u64;
        let t0 = Instant::now();
        if graph.sw_fraction > 0.0 {
            if graph.sw_aggregate {
                self.component_edges.clear();
                for (u, neighbors) in graph.weak.iter().enumerate() {
                    for &(v, j) in neighbors {
                        if u >= v || self.label[u] == self.label[v] {
                            continue;
                        }
                        let (a, b) = (
                            self.label[u].min(self.label[v]),
                            self.label[u].max(self.label[v]),
                        );
                        let key = ((a as u64) << 32) | b as u64;
                        self.component_edges.push((
                            key,
                            i64::from(j) * i64::from(state.spins[u]) * i64::from(state.spins[v]),
                        ));
                    }
                }
                self.component_edges.sort_unstable_by_key(|&(key, _)| key);
                let mut i = 0;
                while i < self.component_edges.len() {
                    let key = self.component_edges[i].0;
                    let mut coupling = 0;
                    while i < self.component_edges.len() && self.component_edges[i].0 == key {
                        coupling += self.component_edges[i].1;
                        i += 1;
                    }
                    if coupling < 0
                        && rng.gen::<f64>()
                            < -(2.0 * beta * graph.sw_fraction * coupling as f64 / 1000.0).exp_m1()
                    {
                        self.join((key >> 32) as usize, (key & 0xffff_ffff) as usize);
                    }
                }
            } else {
                for edges in &graph.weak_units {
                    let (u, v, _) = edges[0];
                    if self.label[u] == self.label[v] {
                        continue;
                    }
                    let coupling: i64 = edges
                        .iter()
                        .map(|&(a, b, j)| {
                            i64::from(j) * i64::from(state.spins[a]) * i64::from(state.spins[b])
                        })
                        .sum();
                    if coupling < 0
                        && rng.gen::<f64>()
                            < -((2.0 * beta * graph.sw_fraction * coupling as f64 / 1000.0)
                                .exp_m1())
                    {
                        self.join(u, v);
                    }
                }
            }
            self.label_components(graph.n);
        }
        let mut largest = 0;
        self.boundary_sums(graph, &state.spins);
        for root in 0..graph.n {
            if self.head[root] == usize::MAX {
                continue;
            }
            largest = largest.max(self.size[root]);
            stats.components += 1;
            let delta = -2 * self.acc[root];
            // Heat bath makes neutral components flip with probability 1/2.
            // Flipping all neutral components deterministically only inverts
            // the full configuration and cannot relax a pure ice system.
            if rng.gen::<f64>()
                < 1.0 / (1.0 + (beta * (1.0 - graph.sw_fraction) * delta as f64 / 1000.0).exp())
            {
                stats.accepted += 1;
                state.weak_energy += delta;
                let mut u = self.head[root];
                while u != usize::MAX {
                    let su = i64::from(state.spins[u]);
                    state.spins[u] *= -1;
                    // Every boundary edge (u, v) changes its contribution to v's component.
                    for &(v, j) in
                        &graph.weak_adj[graph.weak_off[u] as usize..graph.weak_off[u + 1] as usize]
                    {
                        let lv = self.label[v as usize];
                        if lv != root {
                            self.acc[lv] -= 2 * i64::from(j) * su * i64::from(state.spins[v as usize]);
                        }
                    }
                    u = self.next[u];
                }
            }
        }
        stats.largest_sum += largest as u64;
        stats.sweeps += 1;
        stats.t_heatbath_ns += t0.elapsed().as_nanos() as u64;
        let t0 = Instant::now();
        state.energy = state.weak_energy + graph.strong_energy(&state.spins);
        stats.t_strong_ns += t0.elapsed().as_nanos() as u64;
    }
}

fn accept(delta: i64, beta: f64, rng: &mut SmallRng) -> bool {
    delta <= 0 || rng.gen::<f64>() < (-beta * delta as f64 / 1000.0).exp()
}

fn unit_sweep(graph: &Graph, state: &mut State, beta: f64, rng: &mut SmallRng) {
    for (a, unit) in graph.units.iter().enumerate() {
        let mut delta = 0_i64;
        let mut weak_delta = 0_i64;
        for &u in unit {
            for &(v, j) in &graph.all[u] {
                if graph.unit_of[v] != a {
                    delta -=
                        2 * i64::from(j) * i64::from(state.spins[u]) * i64::from(state.spins[v]);
                }
            }
            for &(v, j) in &graph.weak[u] {
                if graph.unit_of[v] != a {
                    weak_delta -=
                        2 * i64::from(j) * i64::from(state.spins[u]) * i64::from(state.spins[v]);
                }
            }
        }
        if accept(delta, beta, rng) {
            for &u in unit {
                state.spins[u] *= -1;
            }
            state.energy += delta;
            state.weak_energy += weak_delta;
        }
    }
}

fn single_sweep(graph: &Graph, state: &mut State, beta: f64, rng: &mut SmallRng) {
    for u in 0..graph.n {
        let field: i32 = graph.all_adj[graph.all_off[u] as usize..graph.all_off[u + 1] as usize]
            .iter()
            .map(|&(v, j)| i32::from(j) * i32::from(state.spins[v as usize]))
            .sum();
        let delta = -2 * i64::from(state.spins[u]) * i64::from(field);
        if accept(delta, beta, rng) {
            let weak_field: i32 = graph.weak_adj
                [graph.weak_off[u] as usize..graph.weak_off[u + 1] as usize]
                .iter()
                .map(|&(v, j)| i32::from(j) * i32::from(state.spins[v as usize]))
                .sum();
            state.weak_energy -= 2 * i64::from(state.spins[u]) * i64::from(weak_field);
            state.spins[u] *= -1;
            state.energy += delta;
        }
    }
}

fn houdayer_units(
    graph: &Graph,
    a: &mut State,
    b: &mut State,
    rng: &mut SmallRng,
    stats: &mut Stats,
) {
    let n = graph.units.len();
    let mut disagree: Vec<_> = graph
        .units
        .iter()
        .map(|unit| a.spins[unit[0]] != b.spins[unit[0]])
        .collect();
    if disagree.iter().filter(|&&x| x).count() > n / 2 {
        for spin in &mut b.spins {
            *spin *= -1;
        }
        for x in &mut disagree {
            *x = !*x;
        }
    }
    let choices: Vec<_> = (0..n).filter(|&u| disagree[u]).collect();
    if choices.is_empty() {
        return;
    }
    let mut stack = vec![choices[rng.gen_range(0..choices.len())]];
    let mut seen = vec![false; n];
    seen[stack[0]] = true;
    let mut i = 0;
    while i < stack.len() {
        for &v in &graph.unit_neighbors[stack[i]] {
            if disagree[v] && !seen[v] {
                seen[v] = true;
                stack.push(v);
            }
        }
        i += 1;
    }
    stats.houdayer_moves += 1;
    stats.houdayer_disagreement += choices
        .iter()
        .map(|&u| graph.units[u].len() as u64)
        .sum::<u64>();
    for unit in stack {
        stats.houdayer_sites += graph.units[unit].len() as u64;
        for &u in &graph.units[unit] {
            a.spins[u] *= -1;
            b.spins[u] *= -1;
        }
    }
    let before = a.energy + b.energy;
    a.energy = graph.energy(&a.spins);
    b.energy = graph.energy(&b.spins);
    debug_assert_eq!(before, a.energy + b.energy);
    a.weak_energy = a.energy - graph.strong_energy(&a.spins);
    b.weak_energy = b.energy - graph.strong_energy(&b.spins);
}

fn houdayer(graph: &Graph, a: &mut State, b: &mut State, rng: &mut SmallRng, stats: &mut Stats) {
    if graph.pair_mode == PairMode::Fixed {
        houdayer_units(graph, a, b, rng, stats);
        return;
    }
    // A unit is eligible only if every member disagrees. Mixed-parity units
    // are expanded to their physical-spin connected component below.
    let mut disagree: Vec<bool> = a.spins.iter().zip(&b.spins).map(|(x, y)| x != y).collect();
    if disagree.iter().filter(|&&x| x).count() > graph.n / 2 {
        for s in &mut b.spins {
            *s *= -1;
        }
        for x in &mut disagree {
            *x = !*x;
        }
    }
    let choices: Vec<_> = (0..graph.n).filter(|&u| disagree[u]).collect();
    if choices.is_empty() {
        return;
    }
    let mut stack = vec![choices[rng.gen_range(0..choices.len())]];
    let mut visited = vec![false; graph.n];
    visited[stack[0]] = true;
    let mut i = 0;
    while i < stack.len() {
        let u = stack[i];
        for &(v, _) in &graph.all[u] {
            if disagree[v] && !visited[v] {
                visited[v] = true;
                stack.push(v);
            }
        }
        i += 1;
    }
    stats.houdayer_moves += 1;
    stats.houdayer_sites += stack.len() as u64;
    stats.houdayer_disagreement += choices.len() as u64;
    for u in stack {
        a.spins[u] *= -1;
        b.spins[u] *= -1;
    }
    let before = a.energy + b.energy;
    a.energy = graph.energy(&a.spins);
    b.energy = graph.energy(&b.spins);
    debug_assert_eq!(before, a.energy + b.energy);
    a.weak_energy = a.energy - graph.strong_energy(&a.spins);
    b.weak_energy = b.energy - graph.strong_energy(&b.spins);
}

fn encode_spins(spins: &[i8]) -> String {
    let mut bytes = vec![0_u8; spins.len().div_ceil(8)];
    for (i, &s) in spins.iter().enumerate() {
        if s == -1 {
            bytes[i / 8] |= 1 << (i % 8);
        }
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Default, Serialize)]
struct Stats {
    exact_blocks: usize,
    exact_rejected: usize,
    exact_max_width: usize,
    exact_max_units: usize,
    exact_updates: usize,
    houdayer_moves: u64,
    houdayer_sites: u64,
    houdayer_disagreement: u64,
    components: u64,
    accepted: u64,
    largest_sum: u64,
    sweeps: u64,
    swaps: u64,
    swaps_accepted: u64,
    t_partition_ns: u64,
    t_heatbath_ns: u64,
    t_strong_ns: u64,
    t_single_ns: u64,
}

#[derive(Serialize)]
struct ResultRow {
    pair_mode: PairMode,
    tile_weight_milli: i32,
    sw_fraction: f64,
    sw_every: usize,
    copies: usize,
    houdayer_every: usize,
    pair_defects: usize,
    tile_defects: usize,
    nonce: String,
    model_hash: String,
    variant: &'static str,
    seed: u64,
    sweeps: usize,
    replicas: usize,
    restarts: usize,
    schedule: Schedule,
    single_every: usize,
    strip_every: usize,
    cluster_every: usize,
    polish_sweeps: usize,
    dimer_weight_milli: i32,
    beta: [f64; 2],
    seconds: f64,
    cpu_s: Option<f64>,
    best_energy: f64,
    best_milli: i64,
    best_spins_hex: String,
    spins_hex: Vec<String>,
    trace: Vec<(usize, i64, f64)>,
    stats: Stats,
}

fn unit_adjacency(graph: &Graph) -> Vec<Vec<usize>> {
    let mut weights = BTreeMap::<(usize, usize), i64>::new();
    for &(u, v, j) in &graph.edges {
        let (a, b) = (graph.unit_of[u], graph.unit_of[v]);
        if a != b {
            *weights.entry((a.min(b), a.max(b))).or_default() += i64::from(j);
        }
    }
    let mut adjacency = vec![Vec::new(); graph.units.len()];
    for ((a, b), j) in weights {
        if j != 0 {
            adjacency[a].push(b);
            adjacency[b].push(a);
        }
    }
    adjacency
}

fn cpu_seconds(ticks: f64) -> Result<f64, String> {
    let stat = std::fs::read_to_string("/proc/self/stat").map_err(|e| e.to_string())?;
    let (_, fields) = stat.rsplit_once(") ").ok_or("invalid /proc/self/stat")?;
    let fields: Vec<_> = fields.split_whitespace().collect();
    let user: f64 = fields
        .get(11)
        .ok_or("missing CPU ticks")?
        .parse::<f64>()
        .map_err(|e| e.to_string())?;
    let system: f64 = fields
        .get(12)
        .ok_or("missing CPU ticks")?
        .parse::<f64>()
        .map_err(|e| e.to_string())?;
    Ok((user + system) / ticks)
}

fn solve(job: Job, args: &Args) -> Result<ResultRow, String> {
    let started = Instant::now();
    let ticks = if args.cpu_limit_s > 0.0 {
        let output = std::process::Command::new("getconf")
            .arg("CLK_TCK")
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err("getconf CLK_TCK failed".into());
        }
        let ticks = String::from_utf8(output.stdout)
            .map_err(|e| e.to_string())?
            .trim()
            .parse::<f64>()
            .map_err(|e| e.to_string())?;
        if !ticks.is_finite() || ticks <= 0.0 {
            return Err("invalid clock tick rate".into());
        }
        ticks
    } else {
        1.0
    };
    let cpu_start = if args.cpu_limit_s > 0.0 {
        cpu_seconds(ticks)?
    } else {
        0.0
    };
    let mut cpu_now = cpu_start;
    let mut graph = Graph::new(&job, args.dimer_weight_milli)?;
    graph.configure(args.tile_weight_milli, args.pair_mode, args.sw_fraction);
    graph.sw_aggregate = args.sw_aggregate;
    if args.pair_mode == PairMode::Fixed && args.houdayer_every > 0 {
        graph.unit_neighbors = unit_adjacency(&graph);
    }
    let exact = if args.exact_passes > 0 || args.exact_every > 0 {
        Some(if args.exact_ball > 0 {
            exact::ExactTail::balls(&graph, args.exact_ball, args.exact_width)?
        } else {
            exact::ExactTail::new(
                &graph,
                &job.physical_nodes,
                args.exact_strips,
                args.exact_width,
            )?
        })
    } else {
        None
    };
    let strips = if args.strip_every > 0 || args.polish_sweeps > 0 {
        Some(strips::StripTopology::new(
            graph.n,
            &job.physical_nodes,
            &graph.edges,
        )?)
    } else {
        None
    };
    let seed = job.seed.wrapping_add(args.seed_offset);
    let mut rng = SmallRng::seed_from_u64(seed);
    let mut best = State::random(&graph, &mut rng);
    if let Some(hex) = &args.init_hex {
        best.spins = decode_spins(hex, graph.n)?;
        best.energy = graph.energy(&best.spins);
        best.weak_energy = best.energy - graph.strong_energy(&best.spins);
    }
    let mut work = Workspace::new(graph.n);
    let mut trace = Vec::new();
    let mut stats = Stats::default();
    if let Some(tail) = &exact {
        stats.exact_blocks = tail.block_count();
        stats.exact_rejected = tail.rejected;
        stats.exact_max_width = tail.max_width;
        stats.exact_max_units = tail.max_units;
    }
    let betas: Vec<f64> = (0..args.replicas)
        .map(|i| {
            let fraction = if args.replicas == 1 {
                1.0
            } else {
                i as f64 / (args.replicas - 1) as f64
            };
            args.beta_min * (args.beta_max / args.beta_min).powf(fraction)
        })
        .collect();
    let probabilities: Vec<_> = betas
        .iter()
        .map(|&b| Breakup::for_graph(b, &graph))
        .collect();
    for restart in 0..args.restarts {
        let mut states: Vec<_> = (0..args.replicas * args.copies)
            .map(|_| State::random(&graph, &mut rng))
            .collect();
        let warm = (restart > 0 && args.warm_restarts) || (restart == 0 && args.init_hex.is_some());
        if warm {
            for state in &mut states {
                state.clone_from(&best);
            }
        }
        for sweep in 0..args.sweeps {
            if args.cpu_limit_s > 0.0 && sweep.is_multiple_of(16) {
                cpu_now = cpu_seconds(ticks)?;
            }
            let fraction = if args.cpu_limit_s > 0.0 {
                ((cpu_now - cpu_start) * args.restarts as f64 / args.cpu_limit_s - restart as f64)
                    .clamp(0.0, 1.0)
            } else {
                (sweep as f64) / (args.sweeps - 1).max(1) as f64
            };
            let low = if warm { args.reheat_beta } else { args.beta_min };
            let anneal_beta = low * (args.beta_max / low).powf(fraction);
            let anneal_probabilities = Breakup::for_graph(anneal_beta, &graph);
            graph.sw_fraction = if (sweep + 1).is_multiple_of(args.sw_every) {
                args.sw_fraction
            } else {
                0.0
            };
            for (i, state) in states.iter_mut().enumerate() {
                let (beta, p) = match args.schedule {
                    Schedule::Pt => (betas[i / args.copies], &probabilities[i / args.copies]),
                    Schedule::Anneal => (anneal_beta, &anneal_probabilities),
                };
                if args.cluster_every > 0 && (sweep + 1).is_multiple_of(args.cluster_every) {
                    work.sweep(&graph, state, beta, p, &mut rng, &mut stats);
                }
                if args.strip_every > 0 && (sweep + 1).is_multiple_of(args.strip_every) {
                    if let Some(topology) = &strips {
                        state.energy += topology.sweep(&mut state.spins, beta, &mut rng);
                        state.weak_energy = state.energy - graph.strong_energy(&state.spins);
                    }
                }
                if args.single_every > 0 && (sweep + 1).is_multiple_of(args.single_every) {
                    let t0 = Instant::now();
                    if args.pair_mode == PairMode::Fixed {
                        unit_sweep(&graph, state, beta, &mut rng);
                    } else {
                        single_sweep(&graph, state, beta, &mut rng);
                    }
                    stats.t_single_ns += t0.elapsed().as_nanos() as u64;
                }
                if args.exact_every > 0 && (sweep + 1).is_multiple_of(args.exact_every) {
                    if let Some(tail) = &exact {
                        tail.sweep(&graph, state, beta, &mut rng);
                        stats.exact_updates += 1;
                    }
                }
                if state.energy < best.energy {
                    best.clone_from(state);
                    trace.push((
                        restart * args.sweeps + sweep + 1,
                        best.energy,
                        started.elapsed().as_secs_f64(),
                    ));
                }
            }
            if args.houdayer_every > 0 && (sweep + 1).is_multiple_of(args.houdayer_every) {
                for i in 0..args.replicas {
                    let (_, tail) = states.split_at_mut(i * args.copies);
                    let (left, right) = tail.split_at_mut(1);
                    houdayer(&graph, &mut left[0], &mut right[0], &mut rng, &mut stats);
                    for state in [&left[0], &right[0]] {
                        if state.energy < best.energy {
                            best.clone_from(state);
                            trace.push((
                                restart * args.sweeps + sweep + 1,
                                best.energy,
                                started.elapsed().as_secs_f64(),
                            ));
                        }
                    }
                }
            }
            if let Schedule::Pt = args.schedule {
                for i in (sweep % 2..args.replicas.saturating_sub(1)).step_by(2) {
                    for copy in 0..args.copies {
                        let (a, b) = (i * args.copies + copy, (i + 1) * args.copies + copy);
                        let log_ratio = (betas[i] - betas[i + 1])
                            * (states[a].energy - states[b].energy) as f64
                            / 1000.0;
                        stats.swaps += 1;
                        if log_ratio >= 0.0 || rng.gen::<f64>() < log_ratio.exp() {
                            states.swap(a, b);
                            stats.swaps_accepted += 1;
                        }
                    }
                }
            }
            if args.cpu_limit_s > 0.0 && fraction >= 1.0 {
                break;
            }
        }
    }
    if let Some(topology) = &strips {
        for polish in 0..args.polish_sweeps {
            let delta = topology.sweep(&mut best.spins, f64::INFINITY, &mut rng);
            if delta > 0 {
                return Err("exact strip minimization increased energy".into());
            }
            best.energy += delta;
            if delta < 0 {
                trace.push((
                    args.restarts * args.sweeps + polish + 1,
                    best.energy,
                    started.elapsed().as_secs_f64(),
                ));
            }
        }
        best.weak_energy = best.energy - graph.strong_energy(&best.spins);
    }
    if let Some(tail) = &exact {
        for pass in 0..args.exact_passes {
            tail.sweep(&graph, &mut best, f64::INFINITY, &mut rng);
            stats.exact_updates += 1;
            trace.push((
                args.restarts * args.sweeps + args.polish_sweeps + pass + 1,
                best.energy,
                started.elapsed().as_secs_f64(),
            ));
        }
    }
    if args.tabu_steps > 0 {
        tabu::improve(&graph, &mut best, args.tabu_steps, &mut rng);
        trace.push((
            args.restarts * args.sweeps + args.polish_sweeps + args.exact_passes + 1,
            best.energy,
            started.elapsed().as_secs_f64(),
        ));
    }
    if best.energy != graph.energy(&best.spins) {
        return Err("final energy cache mismatch".into());
    }
    let best_spins_hex = encode_spins(&best.spins);
    let pair_defects = graph
        .pairs
        .iter()
        .filter(|&&[u, v]| best.spins[u] != best.spins[v])
        .count();
    let tile_defects = graph
        .tiles
        .iter()
        .filter(|t| {
            (0..4)
                .map(|k| i32::from(t.gauge[k] * best.spins[t.nodes[k]]))
                .sum::<i32>()
                != 0
        })
        .count();
    Ok(ResultRow {
        pair_mode: args.pair_mode,
        tile_weight_milli: args.tile_weight_milli,
        sw_fraction: args.sw_fraction,
        sw_every: args.sw_every,
        copies: args.copies,
        houdayer_every: args.houdayer_every,
        pair_defects,
        tile_defects,
        nonce: job.nonce,
        model_hash: job.model_hash,
        variant: "pair-unit-cluster",
        seed,
        sweeps: args.sweeps,
        replicas: args.replicas,
        restarts: args.restarts,
        schedule: args.schedule,
        single_every: args.single_every,
        strip_every: args.strip_every,
        cluster_every: args.cluster_every,
        polish_sweeps: args.polish_sweeps,
        dimer_weight_milli: args.dimer_weight_milli,
        beta: [args.beta_min, args.beta_max],
        seconds: started.elapsed().as_secs_f64(),
        cpu_s: if args.cpu_limit_s > 0.0 {
            Some(cpu_seconds(ticks)? - cpu_start)
        } else {
            None
        },
        best_energy: best.energy as f64 / 1000.0,
        best_milli: best.energy,
        best_spins_hex: best_spins_hex.clone(),
        spins_hex: vec![best_spins_hex],
        trace,
        stats,
    })
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    if !args.cpu_limit_s.is_finite()
        || args.cpu_limit_s < 0.0
        || !args.reheat_beta.is_finite()
        || args.reheat_beta <= 0.0
        || args.reheat_beta > args.beta_max
        || !(1..=1000).contains(&args.tile_weight_milli)
        || !(0.0..=1.0).contains(&args.sw_fraction)
        || args.copies == 0
        || args.sw_every == 0
        || (args.houdayer_every > 0 && args.copies != 2)
        || (args.pair_mode == PairMode::Fixed && (args.strip_every > 0 || args.polish_sweeps > 0))
        || args.sweeps == 0
        || args.replicas == 0
        || args.restarts == 0
        || (args.strip_every == 0 && args.cluster_every == 0)
        || !args.beta_min.is_finite()
        || !args.beta_max.is_finite()
        || args.beta_min <= 0.0
        || args.beta_max < args.beta_min
        || !(0..=1000).contains(&args.dimer_weight_milli)
    {
        return Err(
            "positive budgets, finite increasing beta endpoints, and dimer weight in 0..=1000 required"
                .into(),
        );
    }
    let input = BufReader::new(File::open(&args.jobs)?);
    let mut output = BufWriter::new(File::create_new(&args.output)?);
    for line in input.lines() {
        let job: Job = serde_json::from_str(&line?)?;
        let result = solve(job, &args)?;
        serde_json::to_writer(&mut output, &result)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_weights_reconstruct_every_tetrahedron_boltzmann_weight() {
        for beta in [0.0_f64, 0.01, 0.4, 1.0, 3.0, 12.0] {
            let [w0, w1, w2] = graph_weights(beta);
            let z = (-2.0 * beta).exp();
            assert!(w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0);
            assert!((w0 - z.powi(4)).abs() < 1e-12);
            assert!((w0 + 3.0 * w1 - z).abs() < 1e-12);
            assert!((w0 + 4.0 * w1 + 2.0 * w2 - 1.0).abs() < 1e-12);
        }
    }

    pub(super) fn fixture() -> Job {
        let tiles = vec![[0, 1, 2, 3], [3, 4, 5, 6]];
        let gauges = [[1, -1, 1, -1], [1, 1, -1, -1]];
        let mut edges = Vec::new();
        let mut j = Vec::new();
        for (tile, gauge) in tiles.iter().zip(gauges) {
            for a in 0..4 {
                for b in a + 1..4 {
                    edges.push([tile[a], tile[b]]);
                    j.push(f64::from(gauge[a] * gauge[b]));
                }
            }
        }
        edges.extend([[0, 4], [2, 5], [1, 6], [0, 7]]);
        j.extend([-1.0, 0.062, -0.125, 0.125]);
        Job {
            nonce: "test".into(),
            model_hash: "test".into(),
            seed: 19,
            h: vec![0.0; 8],
            edges,
            j,
            clusters: tiles,
            pairs: vec![[0, 4]],
            physical_nodes: vec![],
        }
    }

    #[test]
    fn cluster_update_relaxes_defects_without_weak_edges() {
        let job = Job {
            nonce: "pure-ice".into(),
            model_hash: "test".into(),
            seed: 1,
            h: vec![0.0; 4],
            edges: vec![[0, 1], [0, 2], [0, 3], [1, 2], [1, 3], [2, 3]],
            j: vec![1.0; 6],
            clusters: vec![[0, 1, 2, 3]],
            pairs: vec![],
            physical_nodes: vec![],
        };
        let graph = Graph::new(&job, 1000).expect("pure ice");
        let mut state = State {
            spins: vec![1; 4],
            weak_energy: 0,
            energy: 6000,
        };
        let mut work = Workspace::new(4);
        let mut rng = SmallRng::seed_from_u64(103);
        let mut stats = Stats::default();
        for _ in 0..100 {
            work.sweep(
                &graph,
                &mut state,
                3.0,
                &Breakup::new(3.0, 1000),
                &mut rng,
                &mut stats,
            );
        }
        assert_eq!(state.energy, -2000);
    }

    #[test]
    fn cluster_and_single_updates_keep_integer_energy_exact() {
        let graph = Graph::new(&fixture(), 1000).expect("valid fixture");
        let mut rng = SmallRng::seed_from_u64(491);
        let mut state = State::random(&graph, &mut rng);
        let mut work = Workspace::new(graph.n);
        let mut stats = Stats::default();
        for beta in [0.1, 0.5, 1.0, 3.0, 16.0] {
            let p = Breakup::new(beta, 1000);
            for _ in 0..200 {
                work.sweep(&graph, &mut state, beta, &p, &mut rng, &mut stats);
                assert_eq!(state.energy, graph.energy(&state.spins));
                single_sweep(&graph, &mut state, beta, &mut rng);
                assert_eq!(state.energy, graph.energy(&state.spins));
            }
        }
    }

    #[test]
    fn each_cluster_delta_matches_direct_weak_energy_difference() {
        let graph = Graph::new(&fixture(), 1000).expect("valid fixture");
        let mut work = Workspace::new(graph.n);
        let mut rng = SmallRng::seed_from_u64(19);
        for bits in 0..256 {
            let s: Vec<i8> = (0..8)
                .map(|i| if bits & (1 << i) == 0 { 1 } else { -1 })
                .collect();
            work.partition(&graph, &s, &Breakup::new(1.0, 1000), &mut rng);
            for root in 0..8 {
                if work.head[root] == usize::MAX {
                    continue;
                }
                let mut candidate = s.clone();
                for (i, spin) in candidate.iter_mut().enumerate() {
                    if work.label[i] == root {
                        *spin *= -1;
                    }
                }
                let direct =
                    graph.energy(&candidate) - graph.strong_energy(&candidate) - graph.energy(&s)
                        + graph.strong_energy(&s);
                assert_eq!(direct, work.weak_delta(&graph, &s, root));
            }
        }
    }

    fn weak_energy_from_adjacency(graph: &Graph, spins: &[i8]) -> i64 {
        let mut energy = 0_i64;
        for (u, neighbors) in graph.weak.iter().enumerate() {
            for &(v, weight) in neighbors {
                if u < v {
                    energy += i64::from(weight) * i64::from(spins[u]) * i64::from(spins[v]);
                }
            }
        }
        energy
    }

    #[test]
    fn dimer_weight_partition_preserves_every_spin_energy() {
        for dimer_weight_milli in [0, 250, 1000] {
            let graph =
                Graph::new(&fixture(), dimer_weight_milli).expect("valid dimer weight split");
            for bits in 0..256 {
                let spins: Vec<i8> = (0..8)
                    .map(|i| if bits & (1 << i) == 0 { 1 } else { -1 })
                    .collect();
                assert_eq!(
                    graph.strong_energy(&spins) + weak_energy_from_adjacency(&graph, &spins),
                    graph.energy(&spins)
                );
            }
        }
    }

    #[test]
    fn dimer_weight_sweeps_keep_energy_caches_exact() {
        for dimer_weight_milli in [0, 250, 1000] {
            let graph =
                Graph::new(&fixture(), dimer_weight_milli).expect("valid dimer weight split");
            let mut rng = SmallRng::seed_from_u64(491);
            let mut state = State::random(&graph, &mut rng);
            let mut work = Workspace::new(graph.n);
            let mut stats = Stats::default();
            let beta = 1.7;
            let probabilities = Breakup::new(beta, dimer_weight_milli);
            for _ in 0..200 {
                work.sweep(
                    &graph,
                    &mut state,
                    beta,
                    &probabilities,
                    &mut rng,
                    &mut stats,
                );
                assert_eq!(state.energy, graph.energy(&state.spins));
                assert_eq!(
                    state.weak_energy,
                    weak_energy_from_adjacency(&graph, &state.spins)
                );
                single_sweep(&graph, &mut state, beta, &mut rng);
                assert_eq!(state.energy, graph.energy(&state.spins));
                assert_eq!(
                    state.weak_energy,
                    weak_energy_from_adjacency(&graph, &state.spins)
                );
            }
        }
    }

    #[test]
    fn dimer_weight_above_physical_coupling_is_rejected() {
        assert!(Graph::new(&fixture(), 1001).is_err());
    }
    #[test]
    fn contraction_keeps_each_pair_in_one_variable() -> Result<(), String> {
        let units = contraction(7, &[[1, 4], [2, 5]])?;
        assert_eq!(units[1], units[4]);
        assert_eq!(units[2], units[5]);
        assert_eq!(units.iter().max(), Some(&4));
        assert!(contraction(7, &[[1, 4], [4, 5]]).is_err());
        Ok(())
    }

    #[test]
    fn contracted_sw_and_unit_moves_preserve_pairs_and_energy() -> Result<(), String> {
        for mode in [PairMode::Fixed, PairMode::Dynamic] {
            for weight in [250, 750, 1000] {
                for fraction in [0.0, 0.3, 1.0] {
                    let mut graph = Graph::new(&fixture(), 1000)?;
                    graph.configure(weight, mode, fraction);
                    let mut rng = SmallRng::seed_from_u64(491);
                    let mut state = State::random(&graph, &mut rng);
                    let mut work = Workspace::new(graph.n);
                    let mut stats = Stats::default();
                    for beta in [0.1, 0.5, 1.0, 3.0, 16.0] {
                        let p = Breakup::new(beta * f64::from(weight) / 1000.0, 1000);
                        for _ in 0..50 {
                            let parity = state.spins[0] * state.spins[4];
                            work.sweep(&graph, &mut state, beta, &p, &mut rng, &mut stats);
                            assert_eq!(parity, state.spins[0] * state.spins[4]);
                            assert_eq!(state.energy, graph.energy(&state.spins));
                            assert_eq!(
                                state.weak_energy,
                                weak_energy_from_adjacency(&graph, &state.spins)
                            );
                            unit_sweep(&graph, &mut state, beta, &mut rng);
                            assert_eq!(parity, state.spins[0] * state.spins[4]);
                            if mode == PairMode::Dynamic {
                                single_sweep(&graph, &mut state, beta, &mut rng);
                            }
                            assert_eq!(state.energy, graph.energy(&state.spins));
                            assert_eq!(
                                state.weak_energy,
                                weak_energy_from_adjacency(&graph, &state.spins)
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn houdayer_keeps_combined_energy_for_all_fixture_states() -> Result<(), String> {
        let graph = Graph::new(&fixture(), 1000)?;
        let mut rng = SmallRng::seed_from_u64(51);
        let mut stats = Stats::default();
        for _ in 0..1000 {
            let mut a = State::random(&graph, &mut rng);
            let mut b = State::random(&graph, &mut rng);
            let before = a.energy + b.energy;
            houdayer(&graph, &mut a, &mut b, &mut rng, &mut stats);
            assert_eq!(before, a.energy + b.energy);
            assert_eq!(a.energy, graph.energy(&a.spins));
            assert_eq!(b.energy, graph.energy(&b.spins));
        }
        Ok(())
    }
    #[test]
    fn exact_unit_block_matches_brute_force() -> Result<(), String> {
        let mut graph = Graph::new(&fixture(), 1000)?;
        graph.configure(1000, PairMode::Fixed, 0.0);
        let mut state = State {
            spins: vec![1; 8],
            energy: 0,
            weak_energy: 0,
        };
        state.energy = graph.energy(&state.spins);
        state.weak_energy = state.energy - graph.strong_energy(&state.spins);
        let mut rng = SmallRng::seed_from_u64(17);
        let expected = (0..128)
            .map(|bits| {
                let spins: Vec<_> = graph
                    .unit_of
                    .iter()
                    .map(|&u| if bits & (1 << u) == 0 { 1 } else { -1 })
                    .collect();
                graph.energy(&spins)
            })
            .min()
            .ok_or("no states")?;
        exact::update_for_test(&graph, &mut state, &mut rng);
        assert_eq!(state.energy, expected);
        assert_eq!(state.energy, graph.energy(&state.spins));
        Ok(())
    }
    #[test]
    fn tabu_unit_blocks_find_fixture_minimum() -> Result<(), String> {
        let mut graph = Graph::new(&fixture(), 1000)?;
        graph.configure(1000, PairMode::Dynamic, 0.0);
        let mut best = State {
            spins: vec![1; 8],
            energy: 0,
            weak_energy: 0,
        };
        best.energy = graph.energy(&best.spins);
        best.weak_energy = best.energy - graph.strong_energy(&best.spins);
        let expected = (0..256)
            .map(|bits| {
                graph.energy(
                    &(0..8)
                        .map(|u| if bits & (1 << u) == 0 { 1 } else { -1 })
                        .collect::<Vec<_>>(),
                )
            })
            .min()
            .ok_or("no states")?;
        let mut rng = SmallRng::seed_from_u64(35);
        tabu::improve(&graph, &mut best, 500, &mut rng);
        assert_eq!(best.energy, expected);
        assert_eq!(best.energy, graph.energy(&best.spins));
        Ok(())
    }
    #[test]
    fn auxiliary_tile_and_pair_splits_preserve_all_fixture_energies() -> Result<(), String> {
        for tile in [250, 500, 1000] {
            for pair in [0, 125, 250, 1000] {
                let mut graph = Graph::new(&fixture(), pair)?;
                graph.configure(tile, PairMode::Soft, 0.0);
                for bits in 0..256 {
                    let spins: Vec<_> = (0..8)
                        .map(|u| if bits & (1 << u) == 0 { 1 } else { -1 })
                        .collect();
                    assert_eq!(
                        graph.energy(&spins),
                        graph.strong_energy(&spins) + weak_energy_from_adjacency(&graph, &spins)
                    );
                }
                let p = Breakup::for_graph(1.7, &graph);
                assert!(
                    (p.dimer_link + (-2.0 * 1.7 * f64::from(pair) / 1000.0).exp_m1()).abs() < 1e-14
                );
            }
        }
        Ok(())
    }
    #[test]
    fn contracted_graph_removes_cancelled_parallel_edges() -> Result<(), String> {
        let graph = Graph::new(&fixture(), 1000)?;
        let adjacency = unit_adjacency(&graph);
        assert!(!adjacency[graph.unit_of[0]].contains(&graph.unit_of[3]));
        Ok(())
    }

    #[test]
    fn houdayer_unit_moves_preserve_pair_alignment_and_combined_energy() -> Result<(), String> {
        let mut graph = Graph::new(&fixture(), 1000)?;
        graph.configure(1000, PairMode::Fixed, 0.0);
        graph.unit_neighbors = unit_adjacency(&graph);
        let mut rng = SmallRng::seed_from_u64(53);
        let mut stats = Stats::default();
        for _ in 0..1000 {
            let mut a = State::random(&graph, &mut rng);
            let mut b = State::random(&graph, &mut rng);
            let before = a.energy + b.energy;
            houdayer(&graph, &mut a, &mut b, &mut rng, &mut stats);
            assert_eq!(a.energy + b.energy, before);
            assert_eq!(a.spins[0], a.spins[4]);
            assert_eq!(b.spins[0], b.spins[4]);
        }
        Ok(())
    }
}
