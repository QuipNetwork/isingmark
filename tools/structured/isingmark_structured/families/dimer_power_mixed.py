"""F6 dimer-power-mixed: nonce-dependent dimers with a mixed RMS-calibrated power-law background.

Port of the suite's ``work/mixed_power.py``. A greedy matching on a random
edge order fixes the dimers at -1000 milli. Each dimer and each unmatched spin
draws a ratio from 4, 5, 6 and 8. The background couplers take a POWER-125
protocol draw, rescaled by the geometric mean of their endpoint ratios and
rounded to the nearest 10 milli with a floor of 10.
"""

from __future__ import annotations

import numpy as np

from isingmark_structured import protocol
from isingmark_structured.topology import SuiteSpec

RATIOS = np.array([4, 5, 6, 8], dtype=np.int64)
ALPHABET = protocol.power_r_alphabet(125)
REFERENCE_RMS_MILLI = float(np.sqrt(np.mean((8 * np.asarray(ALPHABET, dtype=np.int64)) ** 2)))


def scale_background(raw: np.ndarray, edges: np.ndarray, ratios: np.ndarray) -> np.ndarray:
    edge_ratio = np.sqrt(ratios[edges[:, 0]] * ratios[edges[:, 1]])
    unrounded = 8 * np.abs(raw) * 1000 / (REFERENCE_RMS_MILLI * edge_ratio)
    magnitude = np.maximum(10, 10 * np.rint(unrounded / 10)).astype(np.int64)
    return np.sign(raw).astype(np.int64) * magnitude


def generate(suite: SuiteSpec, nonce: str) -> dict[str, np.ndarray]:
    rng = protocol.nonce_rng(nonce)
    edges = suite.dense_edges.copy()
    order = rng.permutation(len(edges))
    mask = np.zeros(len(edges), dtype=bool)
    mask[order] = protocol.dimer_matching(edges[order])
    pairs = edges[mask].copy()
    pair_ratios = rng.choice(RATIOS, size=len(pairs))
    node_ratios = np.zeros(suite.n_spins, dtype=np.int64)
    node_ratios[pairs[:, 0]] = pair_ratios
    node_ratios[pairs[:, 1]] = pair_ratios
    unmatched = node_ratios == 0
    node_ratios[unmatched] = rng.choice(RATIOS, size=int(unmatched.sum()))
    background = edges[~mask]
    raw = protocol.draw_couplings(nonce, suite.n_spins, len(background), ALPHABET)
    couplings = np.full(len(edges), -1000, dtype=np.int64)
    couplings[~mask] = scale_background(raw, background, node_ratios)
    if np.any(np.abs(couplings[~mask]) >= 1000):
        raise ValueError("background reaches the fixed pair coefficient")
    return {
        "edges": edges,
        "j_milli": couplings,
        "h_milli": np.zeros(suite.n_spins, dtype=np.int64),
        "pairs": pairs,
        "clusters": pairs.copy(),
        "pair_mask": mask.astype(np.int64),
        "pair_ratios": pair_ratios,
        "node_ratios": node_ratios,
        "raw_background_milli": raw,
        "physical_nodes": suite.nodes.copy(),
    }
