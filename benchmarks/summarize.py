"""Summarize native measurements, charging failures the recorded time budget."""
import argparse
import json
import statistics
from collections import defaultdict
from pathlib import Path


def cost(record):
    if record['within_budget']:
        return min(record['wall_ms'], record['budget_ms'])
    return record['budget_ms']


def main(path):
    if path.is_dir():
        path = path / 'results.jsonl'
    variants = defaultdict(dict)
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        record = json.loads(line)
        cases = variants[record['variant']]
        key = record['puzzle_sha256']
        if key in cases:
            raise ValueError(f'Duplicate puzzle for {record["variant"]}: {key}')
        cases[key] = record
    if not variants:
        raise ValueError('No measurements in input')

    summary = {}
    for variant, cases in variants.items():
        solved = sum(record['within_budget'] for record in cases.values())
        summary[variant] = {
            'total': len(cases),
            'solved': solved,
            'failures': len(cases) - solved,
            'capped_mean_s': statistics.mean(cost(r) for r in cases.values()) / 1000,
        }
    if 'baseline' in variants and 'candidate' in variants:
        baseline, candidate = variants['baseline'], variants['candidate']
        keys = baseline.keys() & candidate.keys()
        if keys:
            for key in keys:
                if baseline[key]['budget_ms'] != candidate[key]['budget_ms']:
                    raise ValueError(f'Paired time budgets differ for puzzle {key}')
            old = sum(cost(baseline[key]) for key in keys)
            new = sum(cost(candidate[key]) for key in keys)
            summary['paired'] = {
                'total': len(keys),
                'time_reduction_percent': 100 * (1 - new / old),
            }
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('path', type=Path, help='Results JSONL file or comparison directory')
    main(parser.parse_args().path)
