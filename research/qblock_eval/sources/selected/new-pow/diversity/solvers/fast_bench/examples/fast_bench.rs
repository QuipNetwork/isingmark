//! Tailored Otsuka cluster anneal for the ice-twisted pair cells.
//!
//! Same algorithm as `pair_bench` in soft pair mode (Otsuka tile bonds, soft
//! pair bonds with a weight split, sequential heat bath of components against
//! the weak layer, Metropolis single-spin sweeps, exact strip polish), with a
//! compact CSR layout and an incremental heat bath: component deltas come from
//! one scatter pass over the weak edges and are patched only when a component
//! flips. Several anneals can run on separate threads inside one process.

use clap::Parser;
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::time::Instant;

#[path = "cluster/strips.rs"]
mod strips;

#[derive(Parser, Debug, Clone)]
struct Args {
    #[arg(long)]
    jobs: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, default_value_t = 8192)]
    sweeps: usize,
    /// Independent anneals per model, spread over `threads`.
    #[arg(long, default_value_t = 1)]
    replicas: usize,
    #[arg(long, default_value_t = 1)]
    threads: usize,
    #[arg(long, default_value_t = 1.0)]
    beta_min: f64,
    #[arg(long, default_value_t = 24.0)]
    beta_max: f64,
    #[arg(long, default_value_t = 4)]
    single_every: usize,
    #[arg(long, default_value_t = 16)]
    polish_sweeps: usize,
    #[arg(long, default_value_t = 1000)]
    dimer_weight_milli: i32,
    /// Auxiliary tile bond weight; the rest of each strong tile edge goes to the weak layer.
    #[arg(long, default_value_t = 1000)]
    tile_weight_milli: i32,
    /// Heat-bath passes over the same bond configuration per cluster sweep.
    #[arg(long, default_value_t = 1)]
    heatbath_passes: usize,
    #[arg(long, default_value_t = 0)]
    seed_offset: u64,
    /// Recompute the energy every sweep and abort on a cache mismatch.
    #[arg(long, default_value_t = false)]
    check: bool,
    /// Report the sweep index at which each anneal first reached this energy (0 = off).
    #[arg(long, default_value_t = 0)]
    target_milli: i64,
    /// Ramp shape: geom (geometric in beta), linbeta (linear in beta), lint (linear in 1/beta).
    #[arg(long, default_value = "geom")]
    schedule: String,
    /// Sweeps held at beta_min before the ramp starts (counted inside `sweeps`).
    #[arg(long, default_value_t = 0)]
    hold_sweeps: usize,
    /// Accumulate per-phase timings (slower; for profiling only).
    #[arg(long, default_value_t = false)]
    profile: bool,
    /// Every this many sweeps the threads exchange states (0 = independent anneals).
    /// Requires replicas == threads.
    #[arg(long, default_value_t = 0)]
    resample_every: usize,
    /// best: every thread restarts from the best state. half: the worse half copies the
    /// better half. pa: population-annealing weights exp(-dbeta * E) with systematic resampling.
    #[arg(long, default_value = "best")]
    resample_mode: String,
}

/// Shared state for lock-step resampling between the anneal threads.
struct Exchange {
    barrier: std::sync::Barrier,
    slots: Vec<std::sync::Mutex<(i64, Vec<i8>)>>,
    every: usize,
    mode: String,
}

impl Exchange {
    /// Called by every thread at the same sweep with the replicas it owns. Returns,
    /// per owned replica, the state to continue from (None = keep its own).
    fn step(
        &self,
        ids: &[usize],
        energies: &[i64],
        states: &[&[i8]],
        dbeta: f64,
        rng: &mut SmallRng,
    ) -> Vec<Option<Vec<i8>>> {
        for ((&id, &e), st) in ids.iter().zip(energies).zip(states) {
            if let Ok(mut slot) = self.slots[id].lock() {
                slot.0 = e;
                slot.1.clear();
                slot.1.extend_from_slice(st);
            }
        }
        self.barrier.wait();
        let pop: Vec<(i64, usize)> = (0..self.slots.len())
            .map(|i| (self.slots[i].lock().map(|x| x.0).unwrap_or(i64::MAX), i))
            .collect();
        let mut order = pop.clone();
        order.sort_unstable();
        let n = order.len();
        let e0 = order[0].0;
        let w: Vec<f64> = pop
            .iter()
            .map(|&(e, _)| (-dbeta * (e - e0) as f64 / 1000.0).exp())
            .collect();
        let total: f64 = w.iter().sum();
        let mut out = Vec::with_capacity(ids.len());
        for &id in ids {
            let source = match self.mode.as_str() {
                "best" => order[0].1,
                "half" => {
                    let rank = order.iter().position(|&(_, i)| i == id).unwrap_or(0);
                    if rank < n.div_ceil(2) { id } else { order[rank - n.div_ceil(2)].1 }
                }
                _ => {
                    // Population annealing weights, systematic resampling: replica `id`
                    // takes the member at stratum `id` of the cumulative weight.
                    let u = (rng.gen::<f64>() + id as f64) / n as f64 * total;
                    let mut acc = 0.0;
                    let mut pick = n - 1;
                    for (i, &wi) in w.iter().enumerate() {
                        acc += wi;
                        if u < acc {
                            pick = i;
                            break;
                        }
                    }
                    pick
                }
            };
            out.push(if source == id {
                None
            } else {
                self.slots[source].lock().ok().map(|x| x.1.clone())
            });
        }
        self.barrier.wait();
        out
    }
}

#[derive(Serialize, Default, Clone)]
struct Profile {
    partition: f64,
    scatter: f64,
    heatbath: f64,
    strong: f64,
    single: f64,
}

fn beta_at(args: &Args, sweep: usize) -> f64 {
    if sweep < args.hold_sweeps {
        return args.beta_min;
    }
    let ramp = (args.sweeps - args.hold_sweeps).saturating_sub(1).max(1) as f64;
    let f = (sweep - args.hold_sweeps) as f64 / ramp;
    match args.schedule.as_str() {
        "linbeta" => args.beta_min + (args.beta_max - args.beta_min) * f,
        "lint" => {
            let t = 1.0 / args.beta_min + (1.0 / args.beta_max - 1.0 / args.beta_min) * f;
            1.0 / t
        }
        _ => args.beta_min * (args.beta_max / args.beta_min).powf(f),
    }
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

#[derive(Serialize)]
struct ResultRow {
    nonce: String,
    model_hash: String,
    variant: &'static str,
    seed: u64,
    sweeps: usize,
    replicas: usize,
    threads: usize,
    single_every: usize,
    polish_sweeps: usize,
    dimer_weight_milli: i32,
    tile_weight_milli: i32,
    heatbath_passes: usize,
    beta: [f64; 2],
    seconds: f64,
    setup_seconds: f64,
    best_milli: i64,
    best_spins_hex: String,
    /// Every anneal's best state (added by the diversity study).
    spins_hex: Vec<String>,
    anneal_energies: Vec<i64>,
    anneal_hit_sweeps: Vec<Option<usize>>,
    anneal_seconds: Vec<f64>,
    resample_every: usize,
    resample_mode: String,
    schedule: String,
    hold_sweeps: usize,
    profile: Profile,
    /// (sweep, energy) at each improvement of the best anneal.
    trace: Vec<(usize, i64)>,
}

struct Tile {
    nodes: [u32; 4],
    gauge: [i8; 4],
}

/// Compressed adjacency: `start[u]..start[u + 1]` indexes `to` and `w`.
struct Csr {
    start: Vec<u32>,
    to: Vec<u32>,
    w: Vec<i16>,
}

impl Csr {
    fn build(n: usize, edges: &[(u32, u32, i16)]) -> Self {
        let mut deg = vec![0_u32; n + 1];
        for &(u, v, _) in edges {
            deg[u as usize + 1] += 1;
            deg[v as usize + 1] += 1;
        }
        for i in 0..n {
            deg[i + 1] += deg[i];
        }
        let mut fill = deg.clone();
        let total = deg[n] as usize;
        let mut to = vec![0_u32; total];
        let mut w = vec![0_i16; total];
        for &(u, v, j) in edges {
            let a = fill[u as usize] as usize;
            to[a] = v;
            w[a] = j;
            fill[u as usize] += 1;
            let b = fill[v as usize] as usize;
            to[b] = u;
            w[b] = j;
            fill[v as usize] += 1;
        }
        Self { start: deg, to, w }
    }
}

struct Graph {
    n: usize,
    tiles: Vec<Tile>,
    pairs: Vec<(u32, u32)>,
    dimer_weight_milli: i32,
    tile_weight_milli: i32,
    /// Weak layer plus the residual pair coupling, as an edge list and as CSR.
    weak_edges: Vec<(u32, u32, i16)>,
    weak: Csr,
    /// Every coupling, for single-spin fields. Strong entries come first in each
    /// row; `strong_end[u]` is the index where the weak entries of row `u` start.
    all: Csr,
    strong_end: Vec<u32>,
    /// Sum of |J| over the weak entries of each row, for the early reject.
    weak_abs: Vec<i32>,
    full_edges: Vec<(usize, usize, i32)>,
}

impl Graph {
    fn new(job: &Job, dimer_weight_milli: i32, tile_weight_milli: i32) -> Result<Self, String> {
        let n = job.h.len();
        if n == 0 || job.h.iter().any(|&h| h != 0.0) {
            return Err("zero-field model required".into());
        }
        if !(0..=1000).contains(&dimer_weight_milli) || !(1..=1000).contains(&tile_weight_milli) {
            return Err("dimer weight in 0..=1000 and tile weight in 1..=1000 required".into());
        }
        let mut full_edges = Vec::with_capacity(job.edges.len());
        let mut weight = std::collections::HashMap::with_capacity(job.edges.len());
        for (&[u, v], &j) in job.edges.iter().zip(&job.j) {
            let jm = (j * 1000.0).round();
            if u >= n || v >= n || u == v || (jm - j * 1000.0).abs() > 1e-8 || jm.abs() > 1000.0 {
                return Err("invalid edge".into());
            }
            weight.insert((u.min(v), u.max(v)), jm as i32);
            full_edges.push((u, v, jm as i32));
        }
        let mut strong = std::collections::HashSet::new();
        let mut tiles = Vec::with_capacity(job.clusters.len());
        for &nodes in &job.clusters {
            let mut gauge = [1_i8; 4];
            for k in 1..4 {
                let key = (nodes[0].min(nodes[k]), nodes[0].max(nodes[k]));
                gauge[k] = weight.get(&key).ok_or("missing tetrahedron edge")?.signum() as i8;
            }
            for a in 0..4 {
                for b in a + 1..4 {
                    let key = (nodes[a].min(nodes[b]), nodes[a].max(nodes[b]));
                    if weight.get(&key) != Some(&(1000 * i32::from(gauge[a] * gauge[b])))
                        || !strong.insert(key)
                    {
                        return Err("tetrahedra must be edge-disjoint gauged K4s".into());
                    }
                }
            }
            tiles.push(Tile { nodes: nodes.map(|x| x as u32), gauge });
        }
        let mut paired = vec![false; n];
        let mut pairs = Vec::with_capacity(job.pairs.len());
        for &[u, v] in &job.pairs {
            let key = (u.min(v), u.max(v));
            if u >= n || v >= n || paired[u] || paired[v] || weight.get(&key) != Some(&-1000)
                || !strong.insert(key)
            {
                return Err("pair must be a disjoint extra -1000 edge".into());
            }
            paired[u] = true;
            paired[v] = true;
            pairs.push((u as u32, v as u32));
        }
        let mut weak_edges = Vec::new();
        let mut strong_edges = Vec::new();
        for &(u, v, j) in &full_edges {
            if strong.contains(&(u.min(v), u.max(v))) {
                strong_edges.push((u as u32, v as u32, j as i16));
            } else {
                if j.abs() >= 1000 {
                    return Err("unclassified strong edge".into());
                }
                weak_edges.push((u as u32, v as u32, j as i16));
            }
        }
        let residual = dimer_weight_milli - 1000;
        if residual != 0 {
            for &(u, v) in &pairs {
                weak_edges.push((u, v, residual as i16));
            }
        }
        let tile_residual = 1000 - tile_weight_milli;
        if tile_residual != 0 {
            for t in &tiles {
                for a in 0..4 {
                    for b in a + 1..4 {
                        let j = tile_residual * i32::from(t.gauge[a] * t.gauge[b]);
                        weak_edges.push((t.nodes[a], t.nodes[b], j as i16));
                    }
                }
            }
        }
        // Build `all` with strong entries first in every row.
        let strong_csr = Csr::build(n, &strong_edges);
        let weak_full: Vec<(u32, u32, i16)> = full_edges
            .iter()
            .filter(|&&(u, v, _)| !strong.contains(&(u.min(v), u.max(v))))
            .map(|&(u, v, j)| (u as u32, v as u32, j as i16))
            .collect();
        let weak_csr = Csr::build(n, &weak_full);
        let mut start = vec![0_u32; n + 1];
        let mut to = Vec::with_capacity(full_edges.len() * 2);
        let mut w = Vec::with_capacity(full_edges.len() * 2);
        let mut strong_end = vec![0_u32; n];
        let mut weak_abs = vec![0_i32; n];
        for u in 0..n {
            for k in strong_csr.start[u] as usize..strong_csr.start[u + 1] as usize {
                to.push(strong_csr.to[k]);
                w.push(strong_csr.w[k]);
            }
            strong_end[u] = to.len() as u32;
            for k in weak_csr.start[u] as usize..weak_csr.start[u + 1] as usize {
                to.push(weak_csr.to[k]);
                w.push(weak_csr.w[k]);
                weak_abs[u] += i32::from(weak_csr.w[k].abs());
            }
            start[u + 1] = to.len() as u32;
        }
        Ok(Self {
            n,
            tiles,
            pairs,
            dimer_weight_milli,
            tile_weight_milli,
            weak: Csr::build(n, &weak_edges),
            weak_edges,
            all: Csr { start, to, w },
            strong_end,
            weak_abs,
            full_edges,
        })
    }

    fn energy(&self, s: &[i8]) -> i64 {
        self.full_edges
            .iter()
            .map(|&(u, v, j)| i64::from(j) * i64::from(s[u]) * i64::from(s[v]))
            .sum()
    }

    fn strong_energy(&self, s: &[i8]) -> i64 {
        let mut e = 0_i64;
        for t in &self.tiles {
            let q: i64 = (0..4)
                .map(|k| i64::from(t.gauge[k] * s[t.nodes[k] as usize]))
                .sum();
            e += i64::from(self.tile_weight_milli) * (q * q - 4) / 2;
        }
        for &(u, v) in &self.pairs {
            e -= i64::from(self.dimer_weight_milli)
                * i64::from(s[u as usize])
                * i64::from(s[v as usize]);
        }
        e
    }
}

struct Breakup {
    zero_ice: f64,
    single_ice: f64,
    zero_defect: f64,
    dimer_link: f64,
}

impl Breakup {
    fn new(beta: f64, dimer_weight_milli: i32, tile_weight_milli: i32) -> Self {
        let bt = beta * f64::from(tile_weight_milli) / 1000.0;
        let z = (-2.0 * bt).exp();
        let w0 = z.powi(4);
        let w1 = (z - w0) / 3.0;
        Self {
            zero_ice: w0,
            single_ice: w0 + 4.0 * w1,
            zero_defect: (-6.0 * bt).exp(),
            dimer_link: -(-2.0 * beta * f64::from(dimer_weight_milli) / 1000.0).exp_m1(),
        }
    }
}

struct Work {
    parent: Vec<u32>,
    size: Vec<u32>,
    label: Vec<u32>,
    head: Vec<u32>,
    next: Vec<u32>,
    delta: Vec<i64>,
    roots: Vec<u32>,
}

const NONE: u32 = u32::MAX;

impl Work {
    fn new(n: usize) -> Self {
        Self {
            parent: vec![0; n],
            size: vec![1; n],
            label: vec![0; n],
            head: vec![NONE; n],
            next: vec![NONE; n],
            delta: vec![0; n],
            roots: Vec::with_capacity(n),
        }
    }

    #[inline]
    fn find(&mut self, mut u: u32) -> u32 {
        while self.parent[u as usize] != u {
            let g = self.parent[self.parent[u as usize] as usize];
            self.parent[u as usize] = g;
            u = g;
        }
        u
    }

    #[inline]
    fn join(&mut self, a: u32, b: u32) {
        let mut a = self.find(a);
        let mut b = self.find(b);
        if a == b {
            return;
        }
        if self.size[a as usize] < self.size[b as usize] {
            std::mem::swap(&mut a, &mut b);
        }
        self.parent[b as usize] = a;
        self.size[a as usize] += self.size[b as usize];
    }

    fn partition(&mut self, g: &Graph, s: &[i8], p: &Breakup, rng: &mut SmallRng) {
        for i in 0..g.n {
            self.parent[i] = i as u32;
            self.size[i] = 1;
        }
        for t in &g.tiles {
            let mut pos = [0_u32; 4];
            let mut neg = [0_u32; 4];
            let (mut np, mut nn) = (0, 0);
            for k in 0..4 {
                if s[t.nodes[k] as usize] * t.gauge[k] > 0 {
                    pos[np] = t.nodes[k];
                    np += 1;
                } else {
                    neg[nn] = t.nodes[k];
                    nn += 1;
                }
            }
            if np == 0 || np == 4 {
                continue;
            }
            // One 64-bit draw per tile: the top 53 bits make the uniform, the low
            // bits pick which spins the bonds touch.
            let x = rng.gen::<u64>();
            let r = (x >> 11) as f64 * (1.0 / (1_u64 << 53) as f64);
            if np == 2 {
                if r < p.zero_ice {
                    continue;
                }
                if r < p.single_ice {
                    self.join(pos[(x & 1) as usize], neg[((x >> 1) & 1) as usize]);
                } else {
                    let c = (x & 1) as usize;
                    self.join(pos[0], neg[c]);
                    self.join(pos[1], neg[1 - c]);
                }
            } else if r >= p.zero_defect {
                self.join(pos[(x & 0xff) as usize % np], neg[((x >> 8) & 0xff) as usize % nn]);
            }
        }
        for &(u, v) in &g.pairs {
            if s[u as usize] == s[v as usize] && rng.gen::<f64>() < p.dimer_link {
                self.join(u, v);
            }
        }
        self.head.fill(NONE);
        self.roots.clear();
        for i in 0..g.n {
            let root = self.find(i as u32);
            self.label[i] = root;
            if self.head[root as usize] == NONE {
                self.roots.push(root);
            }
            self.next[i] = self.head[root as usize];
            self.head[root as usize] = i as u32;
        }
    }

    /// One cluster sweep. Returns the weak-energy change.
    fn sweep(
        &mut self,
        g: &Graph,
        s: &mut [i8],
        beta: f64,
        p: &Breakup,
        rng: &mut SmallRng,
        passes: usize,
        prof: Option<&mut Profile>,
    ) -> i64 {
        let t0 = prof.as_ref().map(|_| Instant::now());
        self.partition(g, s, p, rng);
        let t1 = t0.map(|_| Instant::now());
        // Scatter pass: delta[c] = -2 sum over cross edges of J s_u s_v.
        for &r in &self.roots {
            self.delta[r as usize] = 0;
        }
        for &(u, v, j) in &g.weak_edges {
            let (lu, lv) = (self.label[u as usize], self.label[v as usize]);
            if lu != lv {
                let t = -2 * i64::from(j) * i64::from(s[u as usize]) * i64::from(s[v as usize]);
                self.delta[lu as usize] += t;
                self.delta[lv as usize] += t;
            }
        }
        let t2 = t0.map(|_| Instant::now());
        let mut change = 0_i64;
        // Sequential heat bath in root index order (same order as the labels' roots
        // appear, which is the lowest member index of each component). Extra passes
        // reuse the bond configuration: joined spins flip together, so every drawn
        // bond stays satisfied and the partition remains a valid sample.
        for idx in 0..self.roots.len() * passes {
            let idx = idx % self.roots.len();
            let root = self.roots[idx];
            let d = self.delta[root as usize];
            let x = beta * d as f64 / 1000.0;
            let accept = if x <= 0.0 {
                rng.gen::<f64>() < 1.0 / (1.0 + x.exp())
            } else {
                let e = (-x).exp();
                rng.gen::<f64>() < e / (1.0 + e)
            };
            if !accept {
                continue;
            }
            change += d;
            // Patch neighbours: every cross edge (u in root, v outside) changes sign.
            let mut u = self.head[root as usize];
            while u != NONE {
                let su = i64::from(s[u as usize]);
                let (a, b) = (g.weak.start[u as usize] as usize, g.weak.start[u as usize + 1] as usize);
                for k in a..b {
                    let v = g.weak.to[k] as usize;
                    let lv = self.label[v];
                    if lv != root {
                        self.delta[lv as usize] += 4 * i64::from(g.weak.w[k]) * su * i64::from(s[v]);
                    }
                }
                s[u as usize] = -s[u as usize];
                u = self.next[u as usize];
            }
            // Flipping back would undo the change, so the component's own delta flips sign.
            self.delta[root as usize] = -d;
        }
        if let (Some(pr), Some(t0), Some(t1), Some(t2)) = (prof, t0, t1, t2) {
            pr.partition += (t1 - t0).as_secs_f64();
            pr.scatter += (t2 - t1).as_secs_f64();
            pr.heatbath += t2.elapsed().as_secs_f64();
        }
        change
    }
}

#[inline]
fn accept(delta: i64, beta: f64, rng: &mut SmallRng) -> bool {
    delta <= 0 || rng.gen::<f64>() < (-beta * delta as f64 / 1000.0).exp()
}

/// Metropolis single-spin sweep. Returns the total energy change.
fn single_sweep(g: &Graph, s: &mut [i8], beta: f64, rng: &mut SmallRng) -> i64 {
    let mut change = 0_i64;
    for u in 0..g.n {
        let (a, m, b) = (
            g.all.start[u] as usize,
            g.strong_end[u] as usize,
            g.all.start[u + 1] as usize,
        );
        let mut field = 0_i32;
        for k in a..m {
            field += i32::from(g.all.w[k]) * i32::from(s[g.all.to[k] as usize]);
        }
        // Exact early reject: the weak field cannot lower the cost below this bound,
        // and an acceptance probability under e^-20 is treated as zero.
        let strong_delta = -2 * i64::from(s[u]) * i64::from(field);
        let bound = strong_delta - 2 * i64::from(g.weak_abs[u]);
        if bound > 0 && beta * bound as f64 > 20_000.0 {
            continue;
        }
        for k in m..b {
            field += i32::from(g.all.w[k]) * i32::from(s[g.all.to[k] as usize]);
        }
        let delta = -2 * i64::from(s[u]) * i64::from(field);
        if accept(delta, beta, rng) {
            s[u] = -s[u];
            change += delta;
        }
    }
    change
}

struct AnnealOut {
    best_energy: i64,
    best_spins: Vec<i8>,
    hit_sweep: Option<usize>,
    seconds: f64,
    profile: Profile,
    trace: Vec<(usize, i64)>,
}

struct Replica {
    id: usize,
    rng: SmallRng,
    s: Vec<i8>,
    energy: i64,
    weak_energy: i64,
    out: AnnealOut,
}

/// Runs the replicas `ids` in lock-step on one thread (one sweep each per step),
/// so a lock-step exchange across threads sees every replica at the same sweep.
fn anneal_group(
    g: &Graph,
    args: &Args,
    base_seed: u64,
    ids: &[usize],
    exchange: Option<&Exchange>,
) -> Result<Vec<AnnealOut>, String> {
    let started = Instant::now();
    let mut reps: Vec<Replica> = ids
        .iter()
        .map(|&id| {
            let mut rng = SmallRng::seed_from_u64(base_seed.wrapping_add(id as u64));
            let s: Vec<i8> = (0..g.n).map(|_| if rng.gen::<bool>() { 1 } else { -1 }).collect();
            let energy = g.energy(&s);
            Replica {
                id,
                rng,
                energy,
                weak_energy: energy - g.strong_energy(&s),
                out: AnnealOut {
                    best_energy: energy,
                    best_spins: s.clone(),
                    hit_sweep: None,
                    seconds: 0.0,
                    profile: Profile::default(),
                    trace: Vec::new(),
                },
                s,
            }
        })
        .collect();
    let mut work = Work::new(g.n);
    let mut beta_at_last_exchange = args.beta_min;
    for sweep in 0..args.sweeps {
        let beta = beta_at(args, sweep);
        if let Some(ex) = exchange {
            if sweep > 0 && sweep % ex.every == 0 {
                let energies: Vec<i64> = reps.iter().map(|r| r.energy).collect();
                let states: Vec<&[i8]> = reps.iter().map(|r| r.s.as_slice()).collect();
                let mut rng = SmallRng::seed_from_u64(base_seed ^ (sweep as u64) ^ (ids[0] as u64) << 32);
                let picks = ex.step(ids, &energies, &states, beta - beta_at_last_exchange, &mut rng);
                for (r, pick) in reps.iter_mut().zip(picks) {
                    if let Some(new_s) = pick {
                        r.s.copy_from_slice(&new_s);
                        r.energy = g.energy(&r.s);
                        r.weak_energy = r.energy - g.strong_energy(&r.s);
                    }
                }
                beta_at_last_exchange = beta;
            }
        }
        let p = Breakup::new(beta, g.dimer_weight_milli, g.tile_weight_milli);
        for r in &mut reps {
            r.weak_energy += work.sweep(
                g,
                &mut r.s,
                beta,
                &p,
                &mut r.rng,
                args.heatbath_passes,
                args.profile.then_some(&mut r.out.profile),
            );
            let ts = args.profile.then(Instant::now);
            r.energy = r.weak_energy + g.strong_energy(&r.s);
            if let Some(ts) = ts {
                r.out.profile.strong += ts.elapsed().as_secs_f64();
            }
            if args.single_every > 0 && (sweep + 1) % args.single_every == 0 {
                let ts = args.profile.then(Instant::now);
                r.energy += single_sweep(g, &mut r.s, beta, &mut r.rng);
                r.weak_energy = r.energy - g.strong_energy(&r.s);
                if let Some(ts) = ts {
                    r.out.profile.single += ts.elapsed().as_secs_f64();
                }
            }
            if args.check && r.energy != g.energy(&r.s) {
                return Err(format!("energy cache mismatch at sweep {sweep} replica {}", r.id));
            }
            if r.energy < r.out.best_energy {
                r.out.best_energy = r.energy;
                r.out.best_spins.copy_from_slice(&r.s);
                r.out.trace.push((sweep + 1, r.energy));
                if args.target_milli != 0 && r.out.hit_sweep.is_none() && r.energy <= args.target_milli {
                    r.out.hit_sweep = Some(sweep + 1);
                }
            }
        }
    }
    let seconds = started.elapsed().as_secs_f64();
    Ok(reps
        .into_iter()
        .map(|mut r| {
            r.out.seconds = seconds;
            r.out
        })
        .collect())
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

fn solve(job: Job, args: &Args) -> Result<ResultRow, String> {
    let started = Instant::now();
    let g = Graph::new(&job, args.dimer_weight_milli, args.tile_weight_milli)?;
    let strips = if args.polish_sweeps > 0 {
        Some(strips::StripTopology::new(g.n, &job.physical_nodes, &g.full_edges)?)
    } else {
        None
    };
    let setup_seconds = started.elapsed().as_secs_f64();
    let base_seed = job.seed.wrapping_add(args.seed_offset);
    let threads = args.threads.clamp(1, args.replicas);
    let exchange = (args.resample_every > 0).then(|| Exchange {
        barrier: std::sync::Barrier::new(threads),
        slots: (0..args.replicas).map(|_| std::sync::Mutex::new((0, Vec::new()))).collect(),
        every: args.resample_every,
        mode: args.resample_mode.clone(),
    });
    let mut outs: Vec<AnnealOut> = Vec::with_capacity(args.replicas);
    std::thread::scope(|scope| -> Result<(), String> {
        let mut handles = Vec::new();
        for t in 0..threads {
            let g = &g;
            let ex = exchange.as_ref();
            handles.push(scope.spawn(move || -> Result<Vec<AnnealOut>, String> {
                let ids: Vec<usize> = (t..args.replicas).step_by(threads).collect();
                anneal_group(g, args, base_seed, &ids, ex)
            }));
        }
        for h in handles {
            outs.extend(h.join().map_err(|_| "anneal thread panicked".to_string())??);
        }
        Ok(())
    })?;
    let mut best_i = 0;
    for (i, o) in outs.iter().enumerate() {
        if o.best_energy < outs[best_i].best_energy {
            best_i = i;
        }
    }
    let mut best = outs[best_i].best_spins.clone();
    let mut best_energy = outs[best_i].best_energy;
    if let Some(topology) = &strips {
        let mut rng = SmallRng::seed_from_u64(base_seed ^ 0x9e37_79b9_7f4a_7c15);
        for _ in 0..args.polish_sweeps {
            let delta = topology.sweep(&mut best, f64::INFINITY, &mut rng);
            if delta > 0 {
                return Err("strip polish increased energy".into());
            }
            best_energy += delta;
        }
    }
    if best_energy != g.energy(&best) {
        return Err("final energy mismatch".into());
    }
    Ok(ResultRow {
        nonce: job.nonce,
        model_hash: job.model_hash,
        variant: "fast-otsuka-cluster",
        seed: base_seed,
        sweeps: args.sweeps,
        replicas: args.replicas,
        threads,
        single_every: args.single_every,
        polish_sweeps: args.polish_sweeps,
        dimer_weight_milli: args.dimer_weight_milli,
        tile_weight_milli: args.tile_weight_milli,
        heatbath_passes: args.heatbath_passes,
        beta: [args.beta_min, args.beta_max],
        seconds: started.elapsed().as_secs_f64(),
        setup_seconds,
        best_milli: best_energy,
        best_spins_hex: encode_spins(&best),
        spins_hex: outs.iter().map(|o| encode_spins(&o.best_spins)).collect(),
        anneal_energies: outs.iter().map(|o| o.best_energy).collect(),
        anneal_hit_sweeps: outs.iter().map(|o| o.hit_sweep).collect(),
        anneal_seconds: outs.iter().map(|o| o.seconds).collect(),
        resample_every: args.resample_every,
        resample_mode: args.resample_mode.clone(),
        schedule: args.schedule.clone(),
        hold_sweeps: args.hold_sweeps,
        profile: outs[best_i].profile.clone(),
        trace: outs[best_i].trace.clone(),
    })
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    if args.sweeps == 0
        || args.heatbath_passes == 0
        || args.hold_sweeps >= args.sweeps
        || !matches!(args.schedule.as_str(), "geom" | "linbeta" | "lint")
        || !matches!(args.resample_mode.as_str(), "best" | "half" | "pa")
        || args.replicas == 0
        || args.threads == 0
        || !(args.beta_min > 0.0 && args.beta_max >= args.beta_min)
    {
        return Err("positive sweeps, replicas, threads and increasing betas required".into());
    }
    let input = BufReader::new(File::open(&args.jobs)?);
    let mut output = BufWriter::new(File::create_new(&args.output)?);
    for line in input.lines() {
        let job: Job = serde_json::from_str(&line?)?;
        let row = solve(job, &args)?;
        serde_json::to_writer(&mut output, &row)?;
        output.write_all(b"\n")?;
    }
    output.flush()?;
    Ok(())
}
