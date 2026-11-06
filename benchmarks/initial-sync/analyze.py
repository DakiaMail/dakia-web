"""Validate full-body completion and compare observed initial-sync throughput."""
import argparse
import json
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument('--results', type=Path, required=True)
p.add_argument('--count', type=int, default=50000)
a = p.parse_args()
pl_text = (a.results / 'pl.log').read_text()
dakia_text = (a.results / 'dakia.log').read_text()
pl = next(row for row in (json.loads(line) for line in pl_text.splitlines() if line.startswith('{')) if row.get('client') == 'pl_mail')
dakia = next(json.loads(line.removeprefix('INITIAL_SYNC ')) for line in dakia_text.splitlines() if line.startswith('INITIAL_SYNC '))
assert 'test result: ok.' in dakia_text and 'STOPPED:' not in dakia_text and 'FAILED' not in dakia_text
assert pl['rows'] == dakia['rows'] == a.count
assert dakia['scheduled'] is True
assert dakia['indexed_bodies'] == dakia['verified_body_tokens'] == dakia['memberships'] == a.count
if 'indexed_bodies' in pl:
    assert pl['indexed_bodies'] == a.count
else:
    # Earlier runs validated the completed database independently through psql.
    proof = list(map(int, (a.results / 'pl-validation.txt').read_text().strip().split('|')))
    assert proof == [a.count, a.count, 1, a.count, a.count, a.count, a.count]
summary = dict(count=a.count, milestone='all bodies downloaded and indexed', throughput_ratio=pl['seconds'] / dakia['seconds'])
for client, measurement in [('pl', pl), ('dakia', dakia)]:
    wire = json.loads((a.results / f'{client}-imap.json').read_text())
    assert wire['rejected'] == [] and wire['body_rows'] >= a.count
    assert not set(wire['commands']).intersection({'UID STORE', 'APPEND', 'UID COPY', 'UID MOVE', 'EXPUNGE'})
    summary[client] = dict(seconds=measurement['seconds'], messages_per_second=a.count / measurement['seconds'], requests=sum(wire['commands'].values()), connections=wire['connections'], received_bytes=wire['response_bytes'], sent_bytes=wire['request_bytes'], header_rows=wire['header_rows'], body_rows=wire['body_rows'], commands=wire['commands'])
summary['target_met'] = summary['throughput_ratio'] > 1.2
if (a.results / 'pl-resources.json').exists() and (a.results / 'dakia-resources.json').exists():
    resources = {client: json.loads((a.results / f'{client}-resources.json').read_text()) for client in ['pl', 'dakia']}
    for client, usage in resources.items():
        assert usage['exit_status'] == 0 and usage['cpu_core_seconds'] > 0
        assert usage['samples'] > 1 and usage['sampling_errors'] == []
        trace = [json.loads(line) for line in (a.results / f'{client}-resources.samples.jsonl').read_text().splitlines()]
        assert len(trace) == usage['samples']
        if client == 'dakia':
            sampled_cpu = (trace[-1]['cpu_total_ns'] - trace[0]['cpu_total_ns']) / 1e9
            assert abs(sampled_cpu - usage['cpu_core_seconds']) < .05 * usage['cpu_core_seconds'] + .5
            assert usage['kernel_peak_resident_bytes'] > 0
        else:
            assert set(usage['components']) == {'dakia-initial-sync-pl', 'dakia-initial-sync-db'}
            assert abs(sum(c['cpu_core_seconds'] for c in usage['components'].values()) - usage['cpu_core_seconds']) < 1e-6
            assert usage['peak_working_set_bytes'] > 0
    summary['resources'] = resources

(a.results / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps(summary, indent=2))
raise SystemExit(0 if summary['target_met'] else 1)
