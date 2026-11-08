#!/usr/bin/env python3
"""Draws between two MSVC rand states: state' = state * 0x343FD + 0x269EC3 mod 2^32."""
import sys

A, C, M = 0x343FD, 0x269EC3, 1 << 32
AINV = pow(A, -1, M)


def forward(state, n):
    for _ in range(n):
        state = (state * A + C) % M
    return state


def back(state):
    return ((state - C) * AINV) % M


def distance(start, end, limit=50_000_000):
    state = start
    for n in range(limit + 1):
        if state == end:
            return n
        state = (state * A + C) % M
    return None


if __name__ == "__main__":
    print(distance(int(sys.argv[1], 16), int(sys.argv[2], 16)))
