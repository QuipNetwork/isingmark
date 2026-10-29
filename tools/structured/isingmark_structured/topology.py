"""The suite's topology spec, the isingmark fixture, and the map between them.

The suite generators work on the live working graph in dense numbering:
spin ``i`` sits on qubit ``nodes[i]`` and the protocol draw walks ``edges`` in
file order. isingmark replays on a fixture of the same chip that can hold more
qubits and couplers. ``FixtureMap`` carries a model from the first to the
second by qubit id, never by position.
"""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass
from pathlib import Path

import numpy as np

DEFAULT_SUITE_SPEC = (
    Path(__file__).resolve().parents[3] / "fixtures" / "suite-live-2026-09-21.spec.json"
)
DEFAULT_SPIN_MILLI = [-1000, 1000]


@dataclass(frozen=True, eq=False)
class SuiteSpec:
    """A quip topology spec as the suite reads it (``quip_miner_dwave.replay.load_spec``).

    ``eq=False`` keeps identity hashing, so derived layouts can be cached per spec.
    """

    path: Path
    sha256: str
    nodes: np.ndarray
    edges: np.ndarray
    dense_edges: np.ndarray

    @property
    def n_spins(self) -> int:
        return len(self.nodes)


def load_suite_spec(path: Path | str = DEFAULT_SUITE_SPEC) -> SuiteSpec:
    """Read a suite topology spec and number its edges densely by node position."""
    path = Path(path)
    data = path.read_bytes()
    raw = json.loads(data)
    nodes = np.asarray(raw["nodes"], dtype=np.int64)
    edges = np.asarray(raw["edges"], dtype=np.int64).reshape(-1, 2)
    index = {int(label): i for i, label in enumerate(nodes)}
    unknown = [int(label) for label in np.unique(edges) if int(label) not in index]
    if unknown:
        raise ValueError(f"edges name nodes the spec does not list: {unknown[:5]}")
    dense = np.array([[index[int(u)], index[int(v)]] for u, v in edges], dtype=np.int64)
    return SuiteSpec(path, hashlib.sha256(data).hexdigest(), nodes, edges, dense.reshape(-1, 2))


@dataclass(frozen=True)
class Fixture:
    """An isingmark topology fixture (``fixtures/<name>.spec.json``)."""

    name: str
    nodes: list[int]
    edges: list[tuple[int, int]]
    allowed_h_milli: list[int]
    allowed_j_milli: list[int]
    allowed_spin_milli: list[int]

    def topology_hash(self) -> str:
        """isingmark's ``TopologySpec::topology_hash``, as hex.

        BLAKE2b-256 of the SCALE encoding of ``(sorted nodes, sorted
        min/max-normalized edges, canonical h set, canonical j set, canonical
        spin set)``. A canonical set is a 0x00 byte and the sorted values as
        big-endian i32.
        """
        nodes = sorted(self.nodes)
        edges = sorted((min(u, v), max(u, v)) for u, v in self.edges)
        encoded = bytearray(_compact(len(nodes)))
        for node in nodes:
            encoded += node.to_bytes(4, "little")
        encoded += _compact(len(edges))
        for u, v in edges:
            encoded += u.to_bytes(4, "little") + v.to_bytes(4, "little")
        for values in (self.allowed_h_milli, self.allowed_j_milli, self.allowed_spin_milli):
            canonical = b"\x00" + b"".join(
                v.to_bytes(4, "big", signed=True) for v in sorted(values)
            )
            encoded += _compact(len(canonical)) + canonical
        return hashlib.blake2b(bytes(encoded), digest_size=32).hexdigest()


def _compact(n: int) -> bytes:
    """SCALE compact encoding of a length."""
    if n < 1 << 6:
        return (n << 2).to_bytes(1, "little")
    if n < 1 << 14:
        return ((n << 2) | 1).to_bytes(2, "little")
    if n < 1 << 30:
        return ((n << 2) | 2).to_bytes(4, "little")
    body = n.to_bytes((n.bit_length() + 7) // 8, "little")
    return bytes([((len(body) - 4) << 2) | 3]) + body


def load_fixture(path: Path | str) -> Fixture:
    path = Path(path)
    raw = json.loads(path.read_text(encoding="utf-8"))
    name = path.name.removesuffix(".spec.json")
    return Fixture(
        name=name,
        nodes=[int(n) for n in raw["nodes"]],
        edges=[(int(u), int(v)) for u, v in raw.get("edges", [])],
        allowed_h_milli=[int(v) for v in raw["allowed_h_milli"]],
        allowed_j_milli=[int(v) for v in raw["allowed_j_milli"]],
        allowed_spin_milli=[int(v) for v in raw.get("allowed_spin_milli") or DEFAULT_SPIN_MILLI],
    )


class FixtureMap:
    """Carries a model in a suite spec's dense numbering onto a fixture's node and edge order.

    Fixture nodes and edges that the suite spec lacks get zero. A suite node or
    edge that the fixture lacks raises ``ValueError`` at construction.
    """

    def __init__(self, suite: SuiteSpec, fixture: Fixture) -> None:
        node_index = {node: i for i, node in enumerate(fixture.nodes)}
        missing_nodes = [int(q) for q in suite.nodes if int(q) not in node_index]
        if missing_nodes:
            raise ValueError(
                f"the fixture {fixture.name} lacks {len(missing_nodes)} qubits of the suite spec, "
                f"first {missing_nodes[:5]}"
            )
        edge_index = {(min(u, v), max(u, v)): i for i, (u, v) in enumerate(fixture.edges)}
        missing_edges = [
            (int(u), int(v))
            for u, v in suite.edges
            if (min(int(u), int(v)), max(int(u), int(v))) not in edge_index
        ]
        if missing_edges:
            raise ValueError(
                f"the fixture {fixture.name} lacks {len(missing_edges)} couplers of the suite "
                f"spec, first {missing_edges[:5]}"
            )
        self.fixture = fixture
        self._qubits = suite.nodes
        self._node_slot = np.asarray([node_index[int(q)] for q in suite.nodes], dtype=np.int64)
        self._edge_index = edge_index

    def map(
        self, edges: np.ndarray, j_milli: np.ndarray, h_milli: np.ndarray
    ) -> tuple[list[int], list[int]]:
        """``(h, j)`` in the fixture's node and edge order, for a model on dense suite spins."""
        h = np.zeros(len(self.fixture.nodes), dtype=np.int64)
        h[self._node_slot] = h_milli
        j = np.zeros(len(self.fixture.edges), dtype=np.int64)
        physical = self._qubits[np.asarray(edges, dtype=np.int64)]
        filled = np.zeros(len(self.fixture.edges), dtype=bool)
        for (u, v), value in zip(physical.tolist(), np.asarray(j_milli).tolist(), strict=True):
            key = (min(u, v), max(u, v))
            slot = self._edge_index.get(key)
            if slot is None:
                raise ValueError(f"the fixture {self.fixture.name} lacks the model coupler {key}")
            if filled[slot]:
                raise ValueError(f"the model sets coupler {key} twice")
            filled[slot] = True
            j[slot] = value
        return h.tolist(), j.tolist()
