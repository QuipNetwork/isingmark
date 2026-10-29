"""Generators for the structured Ising families of the QPU suite.

Every family is a deterministic function of a 32-byte nonce and the suite's
live topology spec. The package regenerates the suite's models bit for bit
and writes them as isingmark instance files.
"""
