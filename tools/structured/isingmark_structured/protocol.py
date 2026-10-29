"""The protocol draw and the shared helpers of the suite generators.

``draw_ising_milli`` is a port of ``quip_protocol::chacha8::draw_ising_milli``:
a ChaCha8 keystream keyed by the nonce, with field ``i`` taking word ``i`` and
coupling ``e`` taking word ``n_nodes + e``, each selecting
``allowed[word % len(allowed)]``. The other helpers are exact ports of
``quip_miner_dwave.regimes``.
"""

from __future__ import annotations

import hashlib

import numpy as np

# "expand 32-byte k" as four little-endian words.
_CONSTANTS = np.asarray([0x61707865, 0x3320646E, 0x79622D32, 0x6B206574], dtype=np.uint32)
_ROUNDS = (
    (0, 4, 8, 12),
    (1, 5, 9, 13),
    (2, 6, 10, 14),
    (3, 7, 11, 15),
    (0, 5, 10, 15),
    (1, 6, 11, 12),
    (2, 7, 8, 13),
    (3, 4, 9, 14),
)

NONCE_SEED = 20260920
SUITE_MODELS = 24


def _rotate(x: np.ndarray, n: int) -> np.ndarray:
    return (x << np.uint32(n)) | (x >> np.uint32(32 - n))


def _blocks(key: np.ndarray, first_block: int, count: int) -> np.ndarray:
    """Keystream blocks ``first_block .. first_block + count``, shape ``(count, 16)``."""
    counters = np.arange(first_block, first_block + count, dtype=np.uint64)
    state = np.empty((16, count), dtype=np.uint32)
    state[0:4] = _CONSTANTS[:, None]
    state[4:12] = key[:, None]
    state[12] = (counters & np.uint64(0xFFFFFFFF)).astype(np.uint32)
    state[13] = (counters >> np.uint64(32)).astype(np.uint32)
    state[14] = 0
    state[15] = 0
    start = state.copy()
    for _ in range(4):
        for a, b, c, d in _ROUNDS:
            state[a] += state[b]
            state[d] = _rotate(state[d] ^ state[a], 16)
            state[c] += state[d]
            state[b] = _rotate(state[b] ^ state[c], 12)
            state[a] += state[b]
            state[d] = _rotate(state[d] ^ state[a], 8)
            state[c] += state[d]
            state[b] = _rotate(state[b] ^ state[c], 7)
    return (state + start).T


def keystream_words(nonce: bytes, first_word: int, count: int) -> np.ndarray:
    """Words ``first_word .. first_word + count`` of the draw stream of ``nonce``, as uint32."""
    if len(nonce) != 32:
        raise ValueError("a nonce contains 32 bytes")
    if count == 0:
        return np.zeros(0, dtype=np.uint32)
    key = np.frombuffer(nonce, dtype="<u4").astype(np.uint32)
    first_block = first_word // 16
    last_block = (first_word + count - 1) // 16
    words = _blocks(key, first_block, last_block - first_block + 1).reshape(-1)
    skip = first_word - 16 * first_block
    return words[skip : skip + count]


def _select(words: np.ndarray, allowed: list[int]) -> np.ndarray:
    if not allowed:
        raise ValueError("allowed-value set is empty; cannot draw an Ising model")
    table = np.asarray(allowed, dtype=np.int64)
    return table[words.astype(np.int64) % len(table)]


def draw_ising_milli(
    nonce: bytes, n_nodes: int, n_edges: int, allowed_h: list[int], allowed_j: list[int]
) -> tuple[np.ndarray, np.ndarray]:
    """``(h, j)`` in integer milli: the protocol model of ``nonce``."""
    h = _select(keystream_words(nonce, 0, n_nodes), allowed_h) if n_nodes else np.zeros(0, np.int64)
    j = (
        _select(keystream_words(nonce, n_nodes, n_edges), allowed_j)
        if n_edges
        else np.zeros(0, np.int64)
    )
    return h, j


def draw_couplings(
    nonce_hex: str, draw_nodes: int, draw_edges: int, alphabet: list[int]
) -> np.ndarray:
    """The drawn couplings of a nonce, int64 milli, as ``regimes.draw_couplings`` returns them.

    The protocol stream draws one field for each of ``draw_nodes`` nodes (from
    ``[0]``) before the couplings, so the node count is part of the draw.
    """
    nonce = bytes.fromhex(nonce_hex.removeprefix("0x"))
    _, j = draw_ising_milli(nonce, draw_nodes, draw_edges, [0], alphabet)
    return j


def make_nonces(count: int, rng_seed: int = NONCE_SEED) -> list[str]:
    """``count`` nonces of 32 bytes, as hex, from one seeded generator."""
    rng = np.random.default_rng(rng_seed)
    return [rng.bytes(32).hex() for _ in range(count)]


def nonce_rng(nonce_hex: str) -> np.random.Generator:
    """The structural generator of a nonce: NumPy ``default_rng`` seeded by big-endian SHA-256."""
    nonce = bytes.fromhex(nonce_hex)
    if len(nonce) != 32:
        raise ValueError("a nonce contains 32 bytes")
    return np.random.default_rng(int.from_bytes(hashlib.sha256(nonce).digest(), "big"))


def power_r_alphabet(r: int) -> list[int]:
    """POWER-r: values -r..r without zero, each repeated about 1000 / |value| times."""
    return [sign * k for k in range(1, r + 1) for sign in (-1, 1) for _ in range(round(1000 / k))]


def cluster_partition(edges: np.ndarray, size: int) -> np.ndarray:
    """Greedy clusters of ``size`` nodes: for each free node in order, the first simple path of
    ``size`` free nodes by depth-first search over the edge list order. Returns the mask of edges
    inside a cluster."""
    neighbors: dict[int, list[int]] = {}
    for u, v in edges.tolist():
        neighbors.setdefault(u, []).append(v)
        neighbors.setdefault(v, []).append(u)
    cluster_of: dict[int, int] = {}

    def grow(path: list[int]) -> list[int] | None:
        if len(path) == size:
            return path
        for w in neighbors.get(path[-1], []):
            if w not in cluster_of and w not in path:
                found = grow([*path, w])
                if found is not None:
                    return found
        return None

    for start in sorted(neighbors):
        if start in cluster_of:
            continue
        path = grow([start])
        if path is not None:
            for node in path:
                cluster_of[node] = start
    return np.array(
        [u in cluster_of and cluster_of[u] == cluster_of.get(v, -1) for u, v in edges.tolist()]
    )


def dimer_matching(edges: np.ndarray) -> np.ndarray:
    """A greedy maximal matching: edges taken in list order while both ends are free."""
    return cluster_partition(edges, 2)
