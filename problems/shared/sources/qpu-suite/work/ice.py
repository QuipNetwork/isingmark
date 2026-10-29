"""Nonce-defined corner-sharing tetrahedra on the live Zephyr topology."""

from collections import Counter, deque
from dataclasses import replace
from functools import lru_cache
import hashlib
from itertools import combinations
import json
from pathlib import Path
import time

from dwave.graphs import zephyr_coordinates
import networkx as nx
import numpy as np
from quip_miner_dwave import regimes
from quip_miner_dwave.replay import load_spec
from scipy.sparse import csr_matrix
from scipy.sparse.csgraph import maximum_bipartite_matching

SPEC = Path("/home/carback1/quip-data/regimes/topology-live-2026-09-21.spec.json")


@lru_cache(maxsize=1)
def topology():
    """Enumerate crossing odd pairs once, preserving the protocol edge order."""
    base = regimes.native_cell("native-pm1", load_spec(SPEC))
    coordinates = zephyr_coordinates(12)
    labels = [coordinates.linear_to_zephyr(int(q)) for q in base.physical_nodes]
    neighbors = [set() for _ in range(base.n_spins)]
    for u, v in base.draw_edges:
        neighbors[u].add(int(v))
        neighbors[v].add(int(u))
    odd = [
        (int(u), int(v))
        for u, v in base.draw_edges
        if labels[u][0] == labels[v][0] and labels[u][3] != labels[v][3]
    ]
    vertical = np.asarray([e for e in odd if labels[e[0]][0] == 0], dtype=np.int64)
    horizontal = np.asarray([e for e in odd if labels[e[0]][0] == 1], dtype=np.int64)
    horizontal_index = {tuple(sorted(e)): i for i, e in enumerate(horizontal)}
    pairs = []
    for i, (u, v) in enumerate(vertical):
        for edge in combinations(sorted(neighbors[u] & neighbors[v]), 2):
            if edge in horizontal_index:
                pairs.append((i, horizontal_index[edge]))
    return base, vertical, horizontal, np.asarray(pairs, dtype=np.int64)


def generate_model(nonce: str, background_milli: int) -> dict[str, np.ndarray]:
    """Return model arrays and the tetrahedra used by the local block attack."""
    if background_milli not in (125, 200):
        raise ValueError("F11 backgrounds are 125 or 200 milli")
    nonce_bytes = bytes.fromhex(nonce)
    if len(nonce_bytes) != 32:
        raise ValueError("a nonce contains 32 bytes")
    rng = np.random.default_rng(
        int.from_bytes(hashlib.sha256(nonce_bytes).digest(), "big")
    )
    base, vertical, horizontal, candidates = topology()
    vorder = rng.permutation(len(vertical))
    horder = rng.permutation(len(horizontal))
    vinverse, hinverse = np.argsort(vorder), np.argsort(horder)
    graph = csr_matrix(
        (
            np.ones(len(candidates), dtype=np.int8),
            (vinverse[candidates[:, 0]], hinverse[candidates[:, 1]]),
        ),
        shape=(len(vertical), len(horizontal)),
    )
    matching = maximum_bipartite_matching(graph, perm_type="column")
    tetrahedra = np.asarray(
        [
            [*vertical[vorder[i]], *horizontal[horder[j]]]
            for i, j in enumerate(matching)
            if j >= 0
        ],
        dtype=np.int64,
    )
    counts = Counter(
        tuple(sorted(edge))
        for tetrahedron in tetrahedra
        for edge in combinations(tetrahedron, 2)
    )
    if max(counts.values()) != 1:
        raise ValueError("tetrahedra share an edge")
    membership = np.bincount(tetrahedra.ravel(), minlength=base.n_spins)
    if membership.max() > 2:
        raise ValueError("a spin belongs to more than two tetrahedra")
    strong = np.asarray([tuple(sorted(edge)) in counts for edge in base.draw_edges])
    if int(strong.sum()) != 6 * len(tetrahedra):
        raise ValueError("a tetrahedron uses a coupler outside the working graph")
    cell = replace(
        base,
        name=f"ice-glass-{background_milli}",
        kernel_label=regimes.CPU_SA,
        alphabet_milli=[-background_milli, background_milli],
        draw_edges=base.draw_edges[~strong],
    )
    couplings = np.full(len(base.draw_edges), 1000, dtype=np.int64)
    couplings[~strong] = regimes.draw_couplings(cell, nonce)
    gauge = rng.choice(np.asarray([-1, 1], dtype=np.int8), size=base.n_spins)
    couplings *= np.prod(gauge[base.draw_edges], axis=1)
    return {
        "edges": base.draw_edges.copy(),
        "j_milli": couplings,
        "h_milli": np.zeros(base.n_spins, dtype=np.int64),
        "tetrahedra": tetrahedra,
        "gauge": gauge,
        "physical_nodes": base.physical_nodes.copy(),
    }


def census(model: dict[str, np.ndarray]) -> dict:
    """Measure signed cycles and a constructive treewidth upper bound."""
    graph = nx.Graph()
    for (u, v), coupling in zip(model["edges"], model["j_milli"], strict=True):
        if abs(int(coupling)) == 1000:
            graph.add_edge(int(u), int(v), preferred=-int(np.sign(coupling)))
    components = sorted((len(c) for c in nx.connected_components(graph)), reverse=True)
    potential = {}
    for start in sorted(graph):
        if start in potential:
            continue
        potential[start] = 1
        pending = deque([start])
        while pending:
            node = pending.popleft()
            for neighbor in sorted(graph[node]):
                if neighbor not in potential:
                    potential[neighbor] = (
                        potential[node] * graph[node][neighbor]["preferred"]
                    )
                    pending.append(neighbor)
    frustrated = sum(
        potential[u] * potential[v] != data["preferred"]
        for u, v, data in graph.edges(data=True)
    )
    cycle_rank = graph.number_of_edges() - len(graph) + len(components)
    reduced = graph.copy()
    pending = deque(node for node in reduced if reduced.degree(node) <= 2)
    reduced_width = 0
    while pending:
        node = pending.popleft()
        if node not in reduced or reduced.degree(node) > 2:
            continue
        adjacent = list(reduced[node])
        reduced_width = max(reduced_width, len(adjacent))
        reduced.remove_node(node)
        if len(adjacent) == 2:
            reduced.add_edge(*adjacent)
        pending.extend(adjacent)
    width = 0
    if reduced:
        width, _ = nx.approximation.treewidth_min_degree(reduced)
    memberships = [[] for _ in model["h_milli"]]
    for i, tetrahedron in enumerate(model["tetrahedra"]):
        for node in tetrahedron:
            memberships[node].append(i)
    tetra_graph = nx.Graph()
    tetra_graph.add_nodes_from(range(len(model["tetrahedra"])))
    tetra_graph.add_edges_from(group for group in memberships if len(group) == 2)
    if not nx.is_bipartite(tetra_graph):
        raise ValueError("tetrahedron network is not bipartite")
    return {
        "strong_edges": graph.number_of_edges(),
        "strong_active_spins": len(graph),
        "strong_component_sizes": components,
        "strong_cycle_rank": cycle_rank,
        "fundamental_frustrated_cycles": frustrated,
        "fundamental_frustrated_fraction": frustrated / cycle_rank
        if cycle_rank
        else 0.0,
        "series_reduced_spins": len(reduced),
        "min_degree_treewidth_upper_bound": max(width, reduced_width),
        "tetrahedra": len(model["tetrahedra"]),
        "spin_membership_counts": dict(Counter(map(len, memberships))),
        "tetra_network_bipartite": True,
        "tetra_network_cycle_rank": tetra_graph.number_of_edges()
        - len(tetra_graph)
        + nx.number_connected_components(tetra_graph),
        "coupling_rms_milli": float(
            np.sqrt(np.mean(model["j_milli"].astype(float) ** 2))
        ),
        "smallest_magnitude_milli": int(np.abs(model["j_milli"]).min()),
    }


def main(family_dir: Path, background_milli: int) -> None:
    """Generate the frozen cohort and verify exact regeneration before saving."""
    nonces = regimes.make_nonces(24, 20260920)
    output = family_dir / "models"
    output.mkdir(parents=True, exist_ok=True)
    (family_dir / "nonces.json").write_text(json.dumps(nonces, indent=2) + "\n")
    rows = []
    for nonce in nonces:
        started = time.monotonic()
        model = generate_model(nonce, background_milli)
        replay = generate_model(nonce, background_milli)
        for name in model:
            if not np.array_equal(model[name], replay[name]):
                raise ValueError(f"model regeneration differs: {nonce}, {name}")
        path = output / f"{nonce}.npz"
        if path.exists():
            with np.load(path, allow_pickle=False) as existing:
                for name in model:
                    if not np.array_equal(model[name], existing[name]):
                        raise ValueError(
                            f"refusing to replace a different model: {path}"
                        )
        else:
            np.savez_compressed(path, allow_pickle=False, **model)
        row = {
            "nonce": nonce,
            **census(model),
            "generation_and_census_s": time.monotonic() - started,
        }
        rows.append(row)
        print(json.dumps(row), flush=True)
    manifest = {
        "family": family_dir.name,
        "background_milli": background_milli,
        "topology_sha256": hashlib.sha256(SPEC.read_bytes()).hexdigest(),
        "generator_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "seed_rule": "numpy default_rng seeded by big-endian sha256(bytes.fromhex(nonce))",
        "models": rows,
    }
    (family_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
