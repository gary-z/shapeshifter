"""Measure one all-core native solve at a time with a total wall-clock deadline."""
import argparse
import hashlib
import json
import os
import signal
import statistics
import subprocess
import time
from pathlib import Path


def verify(puzzle, placements):
    assert len(placements) == len(puzzle['pieces'])
    board = [row[:] for row in puzzle['board']]
    for piece, (row, column) in zip(puzzle['pieces'], placements):
        assert 0 <= row <= puzzle['rows'] - len(piece)
        assert 0 <= column <= puzzle['columns'] - len(piece[0])
        for r, cells in enumerate(piece):
            for c, active in enumerate(cells):
                if active:
                    board[row+r][column+c] = (board[row+r][column+c] - 1) % puzzle['m']
    assert not any(value for row in board for value in row)


def main(args):
    puzzles = [json.loads(line) for line in args.puzzles.read_text().splitlines() if line.strip()]
    if not puzzles:
        raise ValueError('No puzzles in input')
    binary_hash = hashlib.sha256(args.binary.read_bytes()).hexdigest()
    output = args.output.open('w')
    successes = 0
    costs = []
    for index, puzzle in enumerate(puzzles):
        start = time.monotonic()
        process = subprocess.Popen([str(args.binary.resolve())], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True,
            env={**os.environ, 'RAYON_NUM_THREADS': str(os.cpu_count())})
        timed_out = False
        try:
            stdout, stderr = process.communicate(json.dumps(puzzle), timeout=args.timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(process.pid, signal.SIGKILL)
            stdout, stderr = process.communicate()
        wall_ms = (time.monotonic() - start) * 1000
        messages = [json.loads(line) for line in stdout.splitlines() if line.strip()]
        record = {'variant': args.variant, 'index': index, 'level': puzzle['level'],
            'seed': puzzle.get('seed'), 'binary_sha256': binary_hash,
            'puzzle_sha256': hashlib.sha256(json.dumps(puzzle, sort_keys=True).encode()).hexdigest(),
            'wall_ms': wall_ms, 'budget_ms': args.timeout * 1000, 'timed_out': timed_out,
            'returncode': process.returncode, 'phase_log': stderr.splitlines()}
        for message in messages:
            record.update(message)
        if timed_out:
            record['solved'] = False
        else:
            assert process.returncode == 0 and 'solved' in record, record
        if record.get('solved'):
            verify(puzzle, record['placements'])
        record['within_budget'] = bool(record.get('solved') and wall_ms <= args.timeout * 1000)
        successes += record['within_budget']
        costs.append(min(wall_ms, args.timeout * 1000) if record['within_budget'] else args.timeout * 1000)
        output.write(json.dumps(record) + '\n')
        output.flush()
        print(json.dumps({key: record.get(key) for key in ['variant', 'index', 'level', 'seed',
            'within_budget', 'wall_ms', 'preparation_ms', 'search_ms', 'nodes']}), flush=True)
    output.close()
    print(json.dumps({'variant': args.variant, 'solved': successes, 'total': len(puzzles),
        'capped_mean_ms': statistics.mean(costs)}), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--puzzles', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--variant', required=True)
    parser.add_argument('--timeout', default=120, type=float)
    main(parser.parse_args())
