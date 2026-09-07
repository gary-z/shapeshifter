import argparse
import json
import math
import statistics
from collections import defaultdict
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument('directory', type=Path)
a = p.parse_args()
rows = defaultdict(dict)
for path in sorted(a.directory.glob('*-*.jsonl')):
    if path.name.endswith('-puzzle.jsonl'):
        continue
    for line in path.read_text().splitlines():
        record = json.loads(line)
        rows[(record['level'],record['seed'])][record['variant']] = record
paired = [(key,v['baseline'],v['candidate']) for key,v in rows.items() if set(v) == {'baseline','candidate'}]
for variant in ['baseline','candidate']:
    cases = [v[variant] for v in rows.values() if variant in v]
    if cases:
        print(variant, 'n', len(cases), 'solved', sum(x['within_budget'] for x in cases), 'capped_mean_s', statistics.mean(min(x['wall_ms'],120000) if x['within_budget'] else 120000 for x in cases)/1000)
if paired:
    baseline = sum(min(b['wall_ms'],120000) if b['within_budget'] else 120000 for _,b,c in paired)
    candidate = sum(min(c['wall_ms'],120000) if c['within_budget'] else 120000 for _,b,c in paired)
    print('complete_pairs', len(paired), 'time_reduction_percent', 100*(1-candidate/baseline))
    for (level, seed),b,c in paired:
        print(level, seed, '%.3f -> %.3f seconds' % (b['wall_ms']/1000,c['wall_ms']/1000), 'solved', b['within_budget'], c['within_budget'])

    historical_timeout_levels = {68,73,75,80,100}
    subgroup = [(key,b,c) for key,b,c in paired if key[0] in historical_timeout_levels]
    if subgroup:
        old = sum(min(b['wall_ms'],120000) if b['within_budget'] else 120000 for _,b,c in subgroup)
        new = sum(min(c['wall_ms'],120000) if c['within_budget'] else 120000 for _,b,c in subgroup)
        print('historical_timeout_subgroup_pairs',len(subgroup),
              'baseline_mean_s',old/len(subgroup)/1000,'candidate_mean_s',new/len(subgroup)/1000,
              'time_reduction_percent',100*(1-new/old))
    solved_pairs = [(b,c) for _,b,c in paired if b['within_budget'] and c['within_budget']]
    if solved_pairs:
        ratio = math.exp(statistics.mean(math.log(c['wall_ms']/b['wall_ms']) for b,c in solved_pairs))
        print('paired_success_geometric_mean_time_reduction_percent',100*(1-ratio))
