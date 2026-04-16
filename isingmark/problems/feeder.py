# SPDX-License-Identifier: AGPL-3.0-or-later
"""ProblemFeeder: pre-generates IsingProblems in background worker processes."""
from __future__ import annotations

import multiprocessing
from collections.abc import Iterator
from concurrent.futures import Future, ProcessPoolExecutor
from typing import Any

from isingmark.models import IsingProblem


def _generate_one(
    generator_name: str,
    seed: int,
    nodes: list[int],
    edges: list[tuple[int, int]],
    kwargs: dict[str, Any],
) -> IsingProblem:
    """Worker function executed in a child process.

    Imports the generator registry fresh in the child so it is safe under
    the 'spawn' start method (no inherited file descriptors or lock state).

    Args:
        generator_name: Registry key for the problem generator.
        seed: RNG seed for the problem instance.
        nodes: Node identifiers.
        edges: Edge tuples.
        kwargs: Extra keyword arguments forwarded to generator.generate().

    Returns:
        A fully constructed IsingProblem.
    """
    # Late import is intentional: the registry must be populated in the worker
    from isingmark.problems import get_generator  # noqa: PLC0415

    gen = get_generator(generator_name)
    return gen.generate(seed=seed, nodes=nodes, edges=edges, **kwargs)


class ProblemFeeder:
    """Pre-generates IsingProblems concurrently using a ProcessPoolExecutor.

    Problems are generated ahead of time and buffered so that the main process
    can consume them without waiting for individual generation calls.

    Args:
        generator: A ProblemGenerator whose ``name`` property identifies it in
            the registry.  The name is passed to the worker — the generator
            object itself is not pickled.
        nodes: Node list forwarded to every generate() call.
        edges: Edge list forwarded to every generate() call.
        seeds: Ordered sequence of seeds.  One problem is produced per seed.
        buffer_size: Maximum number of in-flight futures kept queued at once.
        max_workers: Number of worker processes in the pool.
        **kwargs: Extra keyword arguments forwarded to generator.generate().
    """

    def __init__(
        self,
        generator: Any,
        nodes: list[int],
        edges: list[tuple[int, int]],
        seeds: list[int],
        buffer_size: int = 8,
        max_workers: int = 2,
        **kwargs: Any,
    ) -> None:
        self._generator_name: str = generator.name
        self._nodes = list(nodes)
        self._edges = list(edges)
        self._seeds = list(seeds)
        self._buffer_size = buffer_size
        self._kwargs = kwargs

        ctx = multiprocessing.get_context("spawn")
        self._pool = ProcessPoolExecutor(max_workers=max_workers, mp_context=ctx)

        self._futures: list[Future[IsingProblem]] = []
        self._seed_iter = iter(self._seeds)
        self._exhausted = False

        self._refill()

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------

    def _submit_one(self) -> bool:
        """Submit the next seed as a future.  Returns False when seeds run out."""
        seed = next(self._seed_iter, None)
        if seed is None:
            self._exhausted = True
            return False
        fut = self._pool.submit(
            _generate_one,
            self._generator_name,
            seed,
            self._nodes,
            self._edges,
            self._kwargs,
        )
        self._futures.append(fut)
        return True

    def _refill(self) -> None:
        """Submit futures until the buffer is full or seeds are exhausted."""
        while not self._exhausted and len(self._futures) < self._buffer_size:
            if not self._submit_one():
                break

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def pop_blocking(self) -> IsingProblem:
        """Return the next problem, blocking until it is ready.

        Raises:
            StopIteration: When all seeds have been consumed.
        """
        self._refill()
        if not self._futures:
            raise StopIteration
        fut = self._futures.pop(0)
        problem = fut.result()
        self._refill()
        return problem

    def try_pop(self) -> IsingProblem | None:
        """Return the next problem if one is ready, else None.

        Returns None when there are no pending futures or when the first
        queued future has not yet completed.
        """
        self._refill()
        if not self._futures:
            return None
        fut = self._futures[0]
        if not fut.done():
            return None
        self._futures.pop(0)
        problem = fut.result()
        self._refill()
        return problem

    def stop(self) -> None:
        """Shut down the worker pool, cancelling any pending futures."""
        for fut in self._futures:
            fut.cancel()
        self._futures.clear()
        self._pool.shutdown(wait=False, cancel_futures=True)

    # ------------------------------------------------------------------
    # Iterator protocol
    # ------------------------------------------------------------------

    def __iter__(self) -> Iterator[IsingProblem]:
        return self

    def __next__(self) -> IsingProblem:
        try:
            return self.pop_blocking()
        except StopIteration:
            raise
