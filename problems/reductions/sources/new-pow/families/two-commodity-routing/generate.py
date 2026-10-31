"""Rank 10: native two-commodity unit routing on a Manhattan-street template.

Template. Every Zephyr cell (c, r) with 1 <= c, r <= 23 is a node. Arcs join
horizontal and vertical neighbours. Row r points right when r is even and left when
r is odd. Column c points up when c is even and down when c is odd. Interior nodes
have two incoming and two outgoing arcs, and the template has directed cycles.

Commodity 1. Every node owns one cell K4 on the row line and column line of layer 1.
Neighbouring nodes share the segment between them, so each arc bit is one spin.

Commodity 2. Every node owns a second cell K4 on two alternating row lines and two
alternating column lines, so neighbouring K4s share no spin. Each arc bit is a
two-spin ferromagnetic chain: the vertical-high corner of the left node and the
horizontal-left corner of the right node for a horizontal arc, and the
horizontal-right corner of the lower node and the vertical-low corner of the upper
node for a vertical arc. The first spin of each chain crosses the commodity-1 spin of
the same arc, and the 496 x1 x2 capacity term sits on that coupler.

Each node and commodity adds 248 (sum out x - sum in x - b)^2 over its existing arcs.
Terminals are drawn independently per commodity, with b = +1 at sources and -1 at
sinks, all on distinct nodes. Each arc bit gets a cost of 62 or 124 milli. Cells with
a missing qubit or coupler are dropped, and their arcs vanish. The template keeps
the largest strongly connected part of what remains.

Usage: generate.py models | shortcut
"""

from __future__ import annotations

import itertools
import sys
from collections import Counter
from functools import lru_cache
from pathlib import Path

import networkx as nx
import numpy as np

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "exact-cover-mixed"))
import famlib as F  # noqa: E402

FAMILY = "two-commodity-routing"
TAG = "two-commodity-routing-v1"
ROW_ID = 10
CONSERVE = 248
CAPACITY = 496
COSTS = (62, 124)
CELLS = range(1, 24)
PAIRS_DIVISOR = 32  # terminal pairs per commodity = nodes // 32
VLO, VHI, HL, HR = 0, 1, 2, 3
PERMS = tuple(itertools.permutations(range(4)))


@lru_cache(maxsize=1)
def layout():
    """Line choices per row and column that keep the most cells alive (nonce independent).

    Row r uses row line rows[r][0] for commodity 1 and rows[r][1 + c % 2] for commodity 2.
    Column c uses cols[c][0] for commodity 1 and cols[c][1 + r % 2] for commodity 2.
    """
    rows = {r: PERMS[0] for r in CELLS}
    cols = {c: PERMS[0] for c in CELLS}

    def alive(c, r, rp, cp):
        return (F.cell_k4(c, r, cp[0], rp[0]) is not None
                and F.cell_k4(c, r, cp[1 + r % 2], rp[1 + c % 2]) is not None)

    changed = True
    while changed:
        changed = False
        for r in CELLS:
            score = lambda p: sum(alive(c, r, p, cols[c]) for c in CELLS)  # noqa: E731
            best = max(PERMS, key=lambda p: (score(p), p == rows[r]))
            if score(best) > score(rows[r]):
                rows[r], changed = best, True
        for c in CELLS:
            score = lambda p: sum(alive(c, r, rows[r], p) for r in CELLS)  # noqa: E731
            best = max(PERMS, key=lambda p: (score(p), p == cols[c]))
            if score(best) > score(cols[c]):
                cols[c], changed = best, True
    return rows, cols


def corners1(c, r):
    rows, cols = layout()
    k, k2 = cols[c][0], rows[r][0]
    return (F.seg(0, c, k, r - 1), F.seg(0, c, k, r), F.seg(1, r, k2, c - 1), F.seg(1, r, k2, c))


def corners2(c, r):
    rows, cols = layout()
    k, k2 = cols[c][1 + r % 2], rows[r][1 + c % 2]
    return (F.seg(0, c, k, r - 1), F.seg(0, c, k, r), F.seg(1, r, k2, c - 1), F.seg(1, r, k2, c))


@lru_cache(maxsize=1)
def template():
    """Nodes and directed arcs, each arc with its spins (nonce independent).

    An arc survives when its spins exist, its chain and capacity couplers exist, and at
    both end nodes its spins share a coupler with the spins of every other surviving
    arc of that node and commodity. Conflicts drop the later arc in cell order. The
    template keeps the largest strongly connected part.
    """
    topo = F.topology()
    arcs = []
    for c in CELLS:
        for r in CELLS:
            options = []
            if c + 1 in CELLS:
                options.append(((c + 1, r), corners1(c, r)[HR], (corners2(c, r)[VHI], corners2(c + 1, r)[HL])))
            if r + 1 in CELLS:
                options.append(((c, r + 1), corners1(c, r)[VHI], (corners2(c, r)[HR], corners2(c, r + 1)[VLO])))
            for w, x1, chain in options:
                if x1 is None or None in chain:
                    continue
                if not (topo.has_edge(*chain) and topo.has_edge(x1, chain[0])):
                    continue
                if w[1] == r:
                    tail, head = ((c, r), w) if r % 2 == 0 else (w, (c, r))
                else:
                    tail, head = ((c, r), w) if c % 2 == 0 else (w, (c, r))
                arcs.append((tail, head, x1, chain))

    def node_spins(arc, v, k):
        t, h, x1, chain = arc
        return x1 if k == 0 else chain[0] if v == min(t, h) else chain[1]

    changed = True
    while changed:
        changed = False
        incident = {}
        for i, arc in enumerate(arcs):
            for v in arc[:2]:
                incident.setdefault(v, []).append(i)
        drop = set()
        for v, ids in sorted(incident.items()):
            for k in range(2):
                for a, i in enumerate(ids):
                    for j in ids[a + 1:]:
                        if i in drop or j in drop:
                            continue
                        if not topo.has_edge(node_spins(arcs[i], v, k), node_spins(arcs[j], v, k)):
                            drop.add(max(i, j))
        if drop:
            arcs = [arc for i, arc in enumerate(arcs) if i not in drop]
            changed = True
    graph = nx.DiGraph()
    graph.add_edges_from((t, h) for t, h, _, _ in arcs)
    core = max(nx.strongly_connected_components(graph), key=len)
    arcs = [arc for arc in arcs if arc[0] in core and arc[1] in core]
    return sorted(core), arcs


def build(nonce: str, attempt: int = 0) -> F.Build:
    topo = F.topology()
    draw = F.Drawer(nonce, f"{TAG}/attempt{attempt}", ROW_ID)
    nodes, arcs = template()
    nindex = {v: i for i, v in enumerate(nodes)}
    graph = nx.DiGraph()
    graph.add_nodes_from(nodes)
    graph.add_edges_from((t, h) for t, h, _, _ in arcs)

    # terminals: distinct nodes in digest order, alternating source and sink per commodity
    order = sorted(nodes, key=lambda v: draw.digest_ids("terminal", v))
    npairs = len(nodes) // PAIRS_DIVISOR
    b = np.zeros((len(nodes), 2), dtype=np.int64)
    pick = iter(order)
    for k in range(2):
        for _ in range(npairs):
            b[nindex[next(pick)], k] = 1
            b[nindex[next(pick)], k] = -1
    feasible = [_single_feasible(graph, nodes, b[:, k]) for k in range(2)]

    energy = F.Energy(topo.n)
    out_arcs = {v: [] for v in nodes}
    in_arcs = {v: [] for v in nodes}
    for a, (t, h, _, _) in enumerate(arcs):
        out_arcs[t].append(a)
        in_arcs[h].append(a)

    def spin(a, k, end):
        """Commodity k spin of arc a inside the K4 of node ``end``."""
        t, h, x1, chain = arcs[a]
        if k == 0:
            return x1
        lo, hi = sorted((t, h))
        return chain[0] if end == lo else chain[1]

    members = []
    for v in nodes:
        for k in range(2):
            coeffs = {}
            for a in out_arcs[v]:
                coeffs[spin(a, k, v)] = 1
            for a in in_arcs[v]:
                coeffs[spin(a, k, v)] = -1
            if coeffs:
                energy.square(CONSERVE, coeffs, -int(b[nindex[v], k]), tag=f"node{k}")
                members.append(tuple(coeffs))
    cost = np.zeros((len(arcs), 2), dtype=np.int64)
    for a, (t, h, x1, chain) in enumerate(arcs):
        energy.pair(x1, chain[0], CAPACITY)
        members.append((x1, chain[0]))
        members.append(chain)
        cd = draw.digest_ids("cost", (nindex[t], nindex[h]))
        for k, s in ((0, x1), (1, chain[0])):
            cost[a, k] = COSTS[F.Drawer.bit(cd, k)]
            energy.lin(s, int(cost[a, k]))

    J, h_, _ = energy.to_ising()
    local = Counter()
    for (i, j), val in J.items():
        local[i] += abs(val)
        local[j] += abs(val)
    strength = max(local[s] + abs(int(h_[s])) for *_, chain in arcs for s in chain) + 1
    if strength > F.MAX_MILLI:
        raise F.Overflow(f"chain strength {strength}")
    for *_, chain in arcs:
        energy.spin_pair(chain[0], chain[1], -strength)

    data_edges = []
    for v in nodes:
        for k in range(2):
            ds = [arcs[a][2] if k == 0 else arcs[a][3][0] for a in out_arcs[v] + in_arcs[v]]
            data_edges += [(ds[i], ds[j]) for i in range(len(ds)) for j in range(i + 1, len(ds))]
    data_edges += [(x1, chain[0]) for *_, x1, chain in arcs]

    extra = {
        "nodes_cell": np.asarray(nodes, dtype=np.int64),
        "node_supply": b,
        "arc_nodes": np.asarray([(nindex[t], nindex[h]) for t, h, _, _ in arcs], dtype=np.int64),
        "arc_spin_commodity1": np.asarray([x1 for *_, x1, _ in arcs], dtype=np.int64),
        "arc_chain_commodity2": np.asarray([chain for *_, chain in arcs], dtype=np.int64),
        "arc_cost_milli": cost,
        "chain_strength_milli": np.asarray(strength),
    }
    deg = Counter((len(out_arcs[v]), len(in_arcs[v])) for v in nodes)
    info = {
        "template_nodes": len(nodes),
        "template_arcs": len(arcs),
        "template_out_in_histogram": {f"{o},{i}": n for (o, i), n in sorted(deg.items())},
        "terminal_pairs_per_commodity": npairs,
        "single_commodity_feasible": feasible,
        "chain_strength_milli": strength,
        "directed_cycles_exist": not nx.is_directed_acyclic_graph(graph),
    }
    if not all(feasible):
        info["reject_reason"] = "infeasible terminals"
    return F.Build(energy, extra, info, members, set(), 2, data_edges)


def _single_feasible(graph, nodes, supply) -> bool:
    flow = nx.DiGraph()
    for t, h in graph.edges:
        flow.add_edge(t, h, capacity=1)
    for v, s in zip(nodes, supply):
        if s > 0:
            flow.add_edge("S", v, capacity=int(s))
        elif s < 0:
            flow.add_edge(v, "T", capacity=int(-s))
    need = int(supply[supply > 0].sum())
    if need == 0:
        return True
    return nx.maximum_flow_value(flow, "S", "T") == need


def defects(result: F.Build, x: np.ndarray) -> dict:
    ex = result.extra
    x1 = x[ex["arc_spin_commodity1"]]
    x2 = x[ex["arc_chain_commodity2"][:, 0]]
    broken = int((x[ex["arc_chain_commodity2"][:, 0]] != x[ex["arc_chain_commodity2"][:, 1]]).sum())
    nn = len(ex["nodes_cell"])
    bad = 0
    for k, xs in ((0, x1), (1, x2)):
        net = np.zeros(nn, dtype=np.int64)
        np.add.at(net, ex["arc_nodes"][:, 0], xs)
        np.add.at(net, ex["arc_nodes"][:, 1], -xs)
        bad += int((net != ex["node_supply"][:, k]).sum())
    return {"capacity_conflicts": int((x1 & x2).sum()), "conservation_violations": bad,
            "used_arcs": [int(x1.sum()), int(x2.sum())], "broken_chains": broken}


def main() -> None:
    cmd = sys.argv[1] if len(sys.argv) > 1 else "models"
    if cmd == "models":
        report = F.generate_all(FAMILY, HERE, build)
        F.capture_request(FAMILY, HERE)
        for m in report["models"]:
            print(m.get("nonce", "")[:12], m.get("active_spins"), m.get("template_nodes"), m.get("template_arcs"),
                  m.get("single_commodity_feasible"), m.get("largest_constraint_component_fraction"),
                  m.get("admission", {}).get("admitted"))
        print("rejected", report["rejected"])
    elif cmd == "shortcut":
        F.shortcut_checks(FAMILY, HERE, build, extra_checks=defects)
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
