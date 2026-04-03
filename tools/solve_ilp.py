#!/usr/bin/env python3
"""Solve a Shapeshifter puzzle using Integer Linear Programming (PuLP).

Based on the approach by juvian: formulate piece placement as an ILP where
each cell's total (initial value + piece hits) must be 0 mod M.

Uses iterative deepening on total flip count (sum of mult[r][c]):
- The minimum flip count is computed analytically from cell values.
- We start by constraining sum(mult) to a narrow range near the minimum,
  then widen if infeasible. This tightens the LP relaxation dramatically
  for the early bands where solutions are most likely.

The simple formulation with parity cuts works best. Things we tried that
made CBC worse or had no benefit:

- Per-row/col min-flips lower bounds: lost 2 solves.
- Skip table symmetry breaking: +2000 constraints, much slower.
- Disaggregated mod: 4x slower.
- GF(M) Gaussian elimination: destroyed sparsity.
- Newer CBC 2.10.11: worse than bundled 2.10.3.
- Native-compiled CBC with parallel: also worse.
- Row/col/diagonal thickness mod-M constraints: worse.
- Google OR-Tools CP-SAT: ~33% solve rate vs CBC's ~78%.
- Z3 SMT solver: 0% on L61+.
- Monte Carlo flip estimation: random placements don't predict solution
  flip counts (off by 2x). Analytical min is much better.

Usage: python3 tools/solve_ilp.py data/puzzle.json [--timeout SECS]
"""

import json
import sys
import time as _time

from pulp import *


def solve_ilp(puzzle_path, max_time=60):
    with open(puzzle_path) as f:
        puz = json.load(f)

    board = puz["board"]
    rows = puz["rows"]
    cols = puz["columns"]
    M = puz["m"]
    pieces_raw = puz["pieces"]
    n = len(pieces_raw)

    pieces = []
    for p in pieces_raw:
        pieces.append([[1 if cell else 0 for cell in row] for row in p])

    placements = []
    for piece in pieces:
        ph, pw = len(piece), len(piece[0])
        placements.append([(r, c) for r in range(rows - ph + 1)
                                   for c in range(cols - pw + 1)])

    # Compute the minimum possible flip count for any valid solution.
    # For cell with value v: minimum hits = (M - v) % M, giving exactly 1 flip
    # (or 0 if v == 0). But pieces can overshoot, causing extra flips.
    # The absolute minimum total flips = sum(ceil_to_M(v) / M) for each cell,
    # where ceil_to_M(v) is the smallest multiple of M >= v... actually:
    # For a valid solution: initial + hits ≡ 0 (mod M).
    # Minimum hits per cell = (M - initial) % M.
    # Flips for that cell at minimum hits = (initial + (M - initial) % M) / M.
    # But total hits = sum of piece cell counts (fixed!) = total_piece_cells.
    # So we can compute: min_total_flips = total_piece_cells_hitting_cells... no,
    # total flips = sum((initial[r][c] + hits[r][c]) / M) for all cells.
    # And sum(hits) = total_piece_cells (constant regardless of placement).
    # So sum(flips) * M = sum(initial) + total_piece_cells.
    # Therefore: total_flips = (sum(initial) + total_piece_cells) / M.
    # This is EXACT — not a bound!

    total_initial = sum(board[r][c] for r in range(rows) for c in range(cols))
    total_piece_cells = sum(sum(cell for row in p for cell in row) for p in pieces)
    # sum(board_val) = total_initial + total_piece_cells = M * sum(mult)
    # So sum(mult) = (total_initial + total_piece_cells) / M
    exact_flips = (total_initial + total_piece_cells) // M
    remainder = (total_initial + total_piece_cells) % M

    if remainder != 0:
        # Impossible: total must be divisible by M for all cells to be 0 mod M.
        # (Each cell contributes a multiple of M to the sum.)
        print(f"Status: Infeasible (total {total_initial + total_piece_cells} not divisible by M={M})")
        print(f"Time: 0.000s")
        print("No solution found.")
        return

    print(f"Total flips must be exactly {exact_flips} "
          f"(initial={total_initial}, piece_cells={total_piece_cells}, M={M})",
          file=sys.stderr)

    def build_and_solve(time_limit):
        """Build ILP with exact flip count and solve."""
        use = [[LpVariable(f"use_{i}_{j}", cat="Binary")
                for j in range(len(placements[i]))]
               for i in range(n)]

        board_val = [[LpVariable(f"bv_{r}_{c}", 0, n + M, cat="Integer")
                      for c in range(cols)]
                     for r in range(rows)]

        mult = [[LpVariable(f"mult_{r}_{c}", 0, (n + M) // M + 1, cat="Integer")
                 for c in range(cols)]
                for r in range(rows)]

        prob = LpProblem("ShapeShifter", LpMinimize)
        prob += 0

        cell_terms = [[[] for _ in range(cols)] for _ in range(rows)]
        for r in range(rows):
            for c in range(cols):
                cell_terms[r][c].append(board[r][c])

        for i, piece in enumerate(pieces):
            ph, pw = len(piece), len(piece[0])
            prob += lpSum(use[i]) == 1
            for j, (pr, pc) in enumerate(placements[i]):
                for dr in range(ph):
                    for dc in range(pw):
                        if piece[dr][dc] == 1:
                            cell_terms[pr + dr][pc + dc].append(use[i][j])

        for r in range(rows):
            for c in range(cols):
                prob += board_val[r][c] == lpSum(cell_terms[r][c])
                prob += board_val[r][c] == mult[r][c] * M

        # Exact total flip count constraint.
        all_mult = [mult[r][c] for r in range(rows) for c in range(cols)]
        prob += lpSum(all_mult) == exact_flips

        # Parity cuts.
        partitions = [
            ("checker", lambda r, c: (r + c) % 2 == 0),
            ("even_row", lambda r, c: r % 2 == 0),
            ("even_col", lambda r, c: c % 2 == 0),
        ]
        for pname, pfunc in partitions:
            group_target = sum((M - board[r][c]) % M
                              for r in range(rows) for c in range(cols) if pfunc(r, c))
            group_hits = []
            for i, piece in enumerate(pieces):
                ph, pw = len(piece), len(piece[0])
                for j, (pr, pc) in enumerate(placements[i]):
                    count = sum(1 for dr in range(ph) for dc in range(pw)
                               if piece[dr][dc] == 1 and pfunc(pr + dr, pc + dc))
                    if count > 0:
                        group_hits.append(count * use[i][j])
            if group_hits:
                max_val = n * 25
                gh = LpVariable(f"gh_{pname}", 0, max_val, cat="Integer")
                gm = LpVariable(f"gm_{pname}", 0, max_val // M + 1, cat="Integer")
                prob += gh == lpSum(group_hits)
                prob += gh == (group_target % M) + M * gm

        start = _time.time()
        solver = PULP_CBC_CMD(msg=0, threads=32, timeLimit=time_limit)
        status = prob.solve(solver)
        elapsed = _time.time() - start

        solution = None
        if LpStatus[status] == "Optimal":
            solution = []
            for i in range(n):
                for j, (pr, pc) in enumerate(placements[i]):
                    if use[i][j].varValue == 1:
                        solution.append((i, pr, pc))
                        break

        return LpStatus[status], elapsed, solution

    status, elapsed, solution = build_and_solve(max_time)

    print(f"Status: {status}")
    print(f"Time: {elapsed:.3f}s")

    if solution is None:
        print("No solution found.")
        return

    print()
    result = [row[:] for row in board]
    for (i, pr, pc) in solution:
        print(f"piece {i}: row {pr}, col {pc}")
        piece = pieces[i]
        for dr in range(len(piece)):
            for dc in range(len(piece[0])):
                if piece[dr][dc] == 1:
                    result[pr + dr][pc + dc] += 1

    all_zero = True
    for r in range(rows):
        row_str = ""
        for c in range(cols):
            v = result[r][c] % M
            row_str += str(v)
            if v != 0:
                all_zero = False
        print(row_str)

    print(f"\nVerification: {'PASS' if all_zero else 'FAIL'}")


if __name__ == "__main__":
    max_time = 60
    puzzle_path = None
    i = 1
    while i < len(sys.argv):
        if sys.argv[i] == "--timeout":
            i += 1
            max_time = int(sys.argv[i])
        else:
            puzzle_path = sys.argv[i]
        i += 1

    if puzzle_path is None:
        print(f"Usage: {sys.argv[0]} puzzle.json [--timeout SECS]")
        sys.exit(1)
    solve_ilp(puzzle_path, max_time)
