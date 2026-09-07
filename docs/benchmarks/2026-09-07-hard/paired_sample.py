"""Run paired all-core solves sequentially, alternating variant order by puzzle."""
import argparse
import hashlib
import json
from pathlib import Path
from types import SimpleNamespace
from run_sample import main as run_sample


def main(args):
    puzzles = args.puzzles.read_text().splitlines()
    args.output.mkdir(parents=True, exist_ok=True)
    for index, puzzle in enumerate(puzzles):
        input_path = args.output / f'{index:03d}-puzzle.jsonl'
        input_path.write_text(puzzle + '\n')
        variants = [('baseline', args.baseline), ('candidate', args.candidate)]
        if index % 2:
            variants.reverse()
        for variant, binary in variants:
            output_path = args.output / f'{index:03d}-{variant}.jsonl'
            if output_path.exists() and output_path.read_text().strip():
                record = json.loads(output_path.read_text())
                assert record['binary_sha256'] == hashlib.sha256(binary.read_bytes()).hexdigest()
                expected = hashlib.sha256(json.dumps(json.loads(puzzle), sort_keys=True).encode()).hexdigest()
                assert record['puzzle_sha256'] == expected
                continue
            run_sample(SimpleNamespace(puzzles=input_path, binary=binary,
                output=output_path, variant=variant, timeout=args.timeout))
    records = []
    for path in sorted(args.output.glob('*-baseline.jsonl')) + sorted(args.output.glob('*-candidate.jsonl')):
        for line in path.read_text().splitlines():
            record = json.loads(line)
            record['index'] = int(path.name.split('-')[0])
            records.append(record)
    (args.output / 'results.jsonl').write_text(''.join(json.dumps(r) + '\n' for r in records))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', required=True, type=Path)
    parser.add_argument('--candidate', required=True, type=Path)
    parser.add_argument('--puzzles', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--timeout', default=120, type=float)
    main(parser.parse_args())
