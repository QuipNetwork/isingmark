"""F12 ice-twisted: independent local gauges on the F11 tetrahedra.

Port of the suite's ``work/ice_twisted.py``. The tetrahedra and the weak
draw are those of ice-glass at 125 milli, ungauged. A second pass of the
nonce generator draws one sign per tetrahedron corner, and each tetrahedron
bond becomes ``1000 * g_a * g_b``, so neighbouring tetrahedra disagree on
the gauge of a shared spin.
"""

from __future__ import annotations

from itertools import combinations

import numpy as np

from isingmark_structured import protocol
from isingmark_structured.families import ice_glass
from isingmark_structured.topology import SuiteSpec


def generate(suite: SuiteSpec, nonce: str) -> dict[str, np.ndarray]:
    reference = ice_glass.generate(suite, nonce, 125)
    vertical, horizontal, _ = ice_glass.odd_pairs(suite)
    rng = protocol.nonce_rng(nonce)
    rng.permutation(len(vertical))
    rng.permutation(len(horizontal))
    tetrahedra = reference["tetrahedra"].copy()
    gauges = rng.choice(np.asarray([-1, 1], dtype=np.int8), size=tetrahedra.shape)
    edges = reference["edges"].copy()
    weights = reference["j_milli"] * np.prod(reference["gauge"][edges], axis=1)
    edge_index = {tuple(sorted(map(int, edge))): i for i, edge in enumerate(edges)}
    for tetrahedron, gauge in zip(tetrahedra, gauges, strict=True):
        for a, b in combinations(range(4), 2):
            index = edge_index[tuple(sorted((int(tetrahedron[a]), int(tetrahedron[b]))))]
            weights[index] = 1000 * int(gauge[a]) * int(gauge[b])
    return {
        "edges": edges,
        "j_milli": weights,
        "h_milli": reference["h_milli"].copy(),
        "physical_nodes": reference["physical_nodes"].copy(),
        "tetrahedra": tetrahedra,
        "local_gauges": gauges,
    }
