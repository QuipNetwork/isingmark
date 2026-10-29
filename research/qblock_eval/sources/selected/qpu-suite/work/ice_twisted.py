"""Independent local gauges on the nonce-defined F11 tetrahedra."""

from collections import deque
import hashlib
from importlib import import_module
from itertools import combinations
from pathlib import Path

import networkx as nx
import numpy as np
from quip_miner_dwave import regimes, replay
import scipy

from work import experiment, ice
from work.experiment import model_hash, write_json


def generate_model(nonce: str) -> dict:
    nonce_bytes = bytes.fromhex(nonce)
    if len(nonce_bytes) != 32:
        raise ValueError("a nonce must contain 32 bytes")
    reference = ice.generate_model(nonce, 125)
    _, vertical, horizontal, _ = ice.topology()
    rng = np.random.default_rng(
        int.from_bytes(hashlib.sha256(nonce_bytes).digest(), "big")
    )
    rng.permutation(len(vertical))
    rng.permutation(len(horizontal))
    tetrahedra = reference["tetrahedra"].copy()
    gauges = rng.choice(np.asarray([-1, 1], dtype=np.int8), size=tetrahedra.shape)
    edges = reference["edges"].copy()
    weights = reference["j_milli"] * np.prod(reference["gauge"][edges], axis=1)
    edge_index = {tuple(sorted(map(int, edge))): i for i, edge in enumerate(edges)}
    for tetrahedron, gauge in zip(tetrahedra, gauges, strict=True):
        for a, b in combinations(range(4), 2):
            index = edge_index[
                tuple(sorted((int(tetrahedron[a]), int(tetrahedron[b]))))
            ]
            weights[index] = 1000 * int(gauge[a]) * int(gauge[b])
    return {
        "edges": edges,
        "j_milli": weights,
        "h_milli": reference["h_milli"].copy(),
        "physical_nodes": reference["physical_nodes"].copy(),
        "tetrahedra": tetrahedra,
        "local_gauges": gauges,
    }


def gauge_obstructions(tetrahedra, gauges, nodes: int) -> dict:
    """Test whether local gauges admit one common physical gauge."""
    membership = [[] for _ in range(nodes)]
    for tetra, (vertices, signs) in enumerate(zip(tetrahedra, gauges, strict=True)):
        for node, sign in zip(vertices, signs, strict=True):
            membership[int(node)].append((tetra, int(sign)))
    links = []
    neighbors = [[] for _ in tetrahedra]
    for group in membership:
        if len(group) > 2:
            raise ValueError("a spin belongs to more than two tetrahedra")
        if len(group) == 2:
            (a, ga), (b, gb) = group
            relation = ga * gb
            links.append((a, b, relation))
            neighbors[a].append((b, relation))
            neighbors[b].append((a, relation))
    potential = {}
    components = 0
    for start in range(len(tetrahedra)):
        if start in potential:
            continue
        components += 1
        potential[start] = 1
        pending = deque([start])
        while pending:
            node = pending.popleft()
            for neighbor, relation in sorted(neighbors[node]):
                if neighbor not in potential:
                    potential[neighbor] = potential[node] * relation
                    pending.append(neighbor)
    frustrated = sum(potential[a] * potential[b] != sign for a, b, sign in links)
    rank = len(links) - len(tetrahedra) + components
    return {
        "tetra_gauge_components": components,
        "tetra_gauge_cycle_rank": rank,
        "frustrated_tetra_fundamental_cycles": frustrated,
        "frustrated_tetra_fundamental_fraction": frustrated / rank if rank else 0.0,
        "globally_gauge_equivalent_to_ice": frustrated == 0,
    }


def main(family_dir: Path) -> None:
    nonces = regimes.make_nonces(24, 20260920)
    output = family_dir / "models"
    output.mkdir(parents=True, exist_ok=True)
    write_json(family_dir / "nonces.json", nonces)
    rows = []
    for nonce in nonces:
        model = generate_model(nonce)
        repeated = generate_model(nonce)
        for key, value in model.items():
            if not np.array_equal(value, repeated[key]):
                raise ValueError(f"model regeneration differs: {nonce}, {key}")
        path = output / f"{nonce}.npz"
        if path.exists():
            with np.load(path, allow_pickle=False) as saved:
                if set(saved.files) != set(model):
                    raise ValueError("saved model fields differ")
                for key, value in model.items():
                    if not np.array_equal(value, saved[key]):
                        raise ValueError(f"saved model differs: {nonce}, {key}")
        else:
            np.savez_compressed(path, **model)
        row = {
            "nonce": nonce,
            "model_hash": model_hash(model),
            **ice.census(model),
            **gauge_obstructions(model["tetrahedra"], model["local_gauges"], 4575),
            "strong_lower_bound_milli": -2000 * len(model["tetrahedra"]),
        }
        rows.append(row)
        print(row, flush=True)
    paths = [
        Path(__file__),
        family_dir / "generate.py",
        Path(ice.__file__),
        Path(experiment.__file__),
        Path(regimes.__file__),
        Path(replay.__file__),
        ice.SPEC,
    ]
    for provider in (
        np.random.Generator,
        np.random.PCG64,
        ice.maximum_bipartite_matching,
        nx.approximation.treewidth_min_degree,
    ):
        location = import_module(provider.__module__).__file__
        if location is None:
            raise ValueError("the provider has no source file")
        paths.append(Path(location))
    write_json(
        family_dir / "manifest.json",
        {
            "family": family_dir.name,
            "background_milli": 125,
            "rng_rule": "SHA256 of nonce bytes, big-endian NumPy default_rng seed. Consume F11 vertical and horizontal permutations, then draw the local gauge array.",
            "weak_draw_rule": "Ungauged signed 125-milli protocol draw on non-tetrahedron native edges.",
            "source_sha256": {
                str(path): hashlib.sha256(path.read_bytes()).hexdigest()
                for path in paths
            },
            "numpy_version": np.__version__,
            "scipy_version": scipy.__version__,
            "networkx_version": nx.__version__,
            "models": rows,
        },
    )
