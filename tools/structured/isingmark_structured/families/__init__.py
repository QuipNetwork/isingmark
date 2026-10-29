"""The registry of ported families, keyed by the suite's cell name."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass, field
from types import ModuleType
from typing import Any

import numpy as np

from isingmark_structured.families import (
    cubic_no_dimer,
    dcl_k44,
    diamond_3d,
    dimer_power_mixed,
    fcl_planted,
    ice_glass,
    ice_twisted,
    strong_net,
    tri_match,
)
from isingmark_structured.topology import SuiteSpec


@dataclass(frozen=True)
class Family:
    """One suite cell: its generator, parameters, and provenance.

    ``code`` is the suite's family label (F1 to F19). ``suite_source`` is the
    suite generator's file name under ``qpu-suite/work/``. ``exact`` records
    whether ``verify.py`` matched all 24 suite models.
    """

    module: ModuleType
    params: dict[str, Any] = field(default_factory=dict)
    tier: int = 1
    code: str = ""
    suite_source: str = ""
    exact: bool = True

    def generate(self, suite: SuiteSpec, nonce: str) -> dict[str, np.ndarray]:
        generator: Callable[..., dict[str, np.ndarray]] = self.module.generate
        return generator(suite, nonce, **self.params)


FAMILIES: dict[str, Family] = {
    "ice-glass-r8": Family(ice_glass, {"background_milli": 125}, 1, "F11", "ice.py"),
    "ice-glass-r5": Family(ice_glass, {"background_milli": 200}, 1, "F11", "ice.py"),
    "ice-twisted": Family(ice_twisted, {}, 2, "F12", "ice_twisted.py"),
    "tri-match-r4": Family(tri_match, {"ratio": 4}, 1, "F1", "trimatch.py"),
    "tri-match-r6": Family(tri_match, {"ratio": 6}, 1, "F1", "trimatch.py"),
    "strong-net-d1.5": Family(strong_net, {"density": 1.5}, 1, "F2", "strongnet.py"),
    "strong-net-d2": Family(strong_net, {"density": 2}, 1, "F2", "strongnet.py"),
    "strong-net-d3": Family(strong_net, {"density": 3}, 1, "F2", "strongnet.py"),
    "diamond-3d": Family(diamond_3d, {}, 1, "F15", "diamond.py"),
    "cubic-no-dimer": Family(cubic_no_dimer, {}, 1, "F16", "cubic.py"),
    "dimer-power-mixed": Family(dimer_power_mixed, {}, 1, "F6", "mixed_power.py"),
    "dcl-k44-l5": Family(dcl_k44, {"cell_milli": 200}, 1, "F5", "dcl.py"),
    "dcl-k44-l7": Family(dcl_k44, {"cell_milli": 143}, 1, "F5", "dcl.py"),
    "fcl-planted-a0.2": Family(fcl_planted, {"density_tenths": 2}, 1, "F4", "fcl.py"),
    "fcl-planted-a0.3": Family(fcl_planted, {"density_tenths": 3}, 1, "F4", "fcl.py"),
}
