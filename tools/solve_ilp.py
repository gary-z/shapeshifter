#!/usr/bin/env python3
"""Solve a Shapeshifter puzzle using Integer Linear Programming (PuLP).

Based on the approach by juvian: formulate piece placement as an ILP where
each cell's total (initial value + piece hits) must be 0 mod M.

Usage: python3 tools/solve_ilp.py data/puzzle.json
"""

import json
import sys
import time as _time

from pulp import *

def solve_ilp(puzzle_path):
    with open(puzzle_path) as f:
        puz = json.load(f)

    board = puz["board"]
    rows = puz["rows"]
    cols = puz["columns"]
    modulo = puz["m"]
    pieces_raw = puz["pieces"]

    n = len(pieces_raw)

    # Convert pieces to list of list of 0/1.
    pieces = []
    for p in pieces_raw:
        pieces.append([[1 if cell else 0 for cell in row] for row in p])

    # Enumerate valid placements for each piece.
    placements = []
    for idx, piece in enumerate(pieces):
        ph = len(piece)
        pw = len(piece[0])
        pl = []
        for r in range(rows - ph + 1):
            for c in range(cols - pw + 1):
                pl.append((r, c))
        placements.append(pl)

    # ILP variables.
    # use[i][j] = 1 if piece i is placed at position j.
    use = [[LpVariable(f"use_{i}_{j}", cat="Binary")
            for j in range(len(placements[i]))]
           for i in range(len(pieces))]

    # board_val[r][c] = total value at cell (initial + piece hits).
    board_val = [[LpVariable(f"bv_{r}_{c}", 0, len(pieces) + modulo, cat="Integer")
                  for c in range(cols)]
                 for r in range(rows)]

    # mult[r][c] = integer multiplier so that board_val == mult * modulo.
    mult = [[LpVariable(f"mult_{r}_{c}", 0, (len(pieces) + modulo) // modulo + 1, cat="Integer")
             for c in range(cols)]
            for r in range(rows)]

    prob = LpProblem("ShapeShifter", LpMinimize)
    prob += 0  # No objective — just satisfying constraints.

    # Build per-cell contribution lists.
    cell_terms = [[[] for _ in range(cols)] for _ in range(rows)]
    for r in range(rows):
        for c in range(cols):
            cell_terms[r][c].append(board[r][c])  # Initial value.

    for i, piece in enumerate(pieces):
        ph = len(piece)
        pw = len(piece[0])
        # Each piece must be placed exactly once.
        prob += lpSum(use[i]) == 1
        for j, (pr, pc) in enumerate(placements[i]):
            for dr in range(ph):
                for dc in range(pw):
                    if piece[dr][dc] == 1:
                        cell_terms[pr + dr][pc + dc].append(use[i][j])

    # Cell constraints.
    for r in range(rows):
        for c in range(cols):
            prob += board_val[r][c] == lpSum(cell_terms[r][c])
            prob += board_val[r][c] == mult[r][c] * modulo

    # --- Parity partition cuts ---
    # For each parity group, total hits on that group must be ≡ target (mod M).
    # These are cheap (2-6 constraints) and cut fractional LP solutions.
    partitions = [
        ("checker", lambda r, c: (r + c) % 2 == 0),
        ("even_row", lambda r, c: r % 2 == 0),
        ("even_col", lambda r, c: c % 2 == 0),
    ]
    for pname, pfunc in partitions:
        # Target: sum of (M - board[r][c]) % M for cells in this group.
        group_target = sum((modulo - board[r][c]) % modulo
                          for r in range(rows) for c in range(cols) if pfunc(r, c))
        # Hits on this group from each placement.
        group_hits = []
        for i, piece in enumerate(pieces):
            ph, pw = len(piece), len(piece[0])
            for j, (pr, pc) in enumerate(placements[i]):
                count = sum(1 for dr in range(ph) for dc in range(pw)
                           if piece[dr][dc] == 1 and pfunc(pr + dr, pc + dc))
                if count > 0:
                    group_hits.append(count * use[i][j])
        if group_hits:
            max_val = sum(t.constant if hasattr(t, 'constant') else 999 for t in group_hits)
            # Safer: just use n * max_piece_size as upper bound.
            max_val = n * 25  # conservative
            gh = LpVariable(f"gh_{pname}", 0, max_val, cat="Integer")
            gm = LpVariable(f"gm_{pname}", 0, max_val // modulo + 1, cat="Integer")
            prob += gh == lpSum(group_hits)
            prob += gh == (group_target % modulo) + modulo * gm

    # Solve.
    start = _time.time()
    solver = PULP_CBC_CMD(msg=0, threads=32)
    status = prob.solve(solver)
    elapsed = _time.time() - start

    print(f"Status: {LpStatus[status]}")
    print(f"Time: {elapsed:.3f}s")

    if LpStatus[status] != "Optimal":
        print("No solution found.")
        return

    # Extract solution.
    print()
    for i, piece in enumerate(pieces):
        for j, (pr, pc) in enumerate(placements[i]):
            if use[i][j].varValue == 1:
                print(f"piece {i}: row {pr}, col {pc}")
                break

    # Verify.
    result = [row[:] for row in board]
    for i, piece in enumerate(pieces):
        for j, (pr, pc) in enumerate(placements[i]):
            if use[i][j].varValue == 1:
                ph = len(piece)
                pw = len(piece[0])
                for dr in range(ph):
                    for dc in range(pw):
                        if piece[dr][dc] == 1:
                            result[pr + dr][pc + dc] += 1

    all_zero = True
    for r in range(rows):
        row_str = ""
        for c in range(cols):
            v = result[r][c] % modulo
            row_str += str(v)
            if v != 0:
                all_zero = False
        print(row_str)

    print(f"\nVerification: {'PASS' if all_zero else 'FAIL'}")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} puzzle.json")
        sys.exit(1)
    solve_ilp(sys.argv[1])
