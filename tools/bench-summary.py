#!/usr/bin/env python3
"""Summarize a Sharpr bench log (JSONL) into the numbers used as baselines.

Usage:
    python3 tools/bench-summary.py [LOG.jsonl]

Without an argument the newest log in ~/.cache/sharpr/logs is used. Compare
two runs by summarizing each; ROADMAP.md acceptance criteria refer to these
figures (folder-open time, main-loop stalls, thumbnail time, memory).
"""
from pathlib import Path
import json
import sys


def quantile(values, fraction):
    if not values:
        return None
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(fraction * len(ordered)))]


def load(path):
    events = []
    for line in path.open():
        line = line.strip()
        if line:
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    return events


def main():
    if len(sys.argv) > 1:
        path = Path(sys.argv[1])
    else:
        logs = sorted((Path.home() / '.cache/sharpr/logs').glob('run-*.jsonl'),
                      key=lambda p: p.stat().st_mtime)
        if not logs:
            raise SystemExit('No bench logs found in ~/.cache/sharpr/logs')
        path = logs[-1]
    events = load(path)
    if not events:
        raise SystemExit(f'{path}: no events')

    def named(name):
        return [e for e in events if e.get('event') == name]

    def field(event, key):
        return event.get('fields', {}).get(key)

    span = (events[-1]['unix_ms'] - events[0]['unix_ms']) / 1000
    print(f'log: {path}')
    print(f'session: {span:.1f} s, {len(events)} events')

    start = named('app.start')
    ready = named('folder.open.ready')
    if start and ready:
        print(f'startup to first folder ready: {ready[0]["unix_ms"] - start[0]["unix_ms"]} ms')

    print('folder opens (request to ready; scan queue wait shown when over 100 ms):')
    pending = {}
    for event in events:
        name = event.get('event')
        folder = field(event, 'path')
        if name == 'folder.open.request':
            pending[folder] = {'request': event['unix_ms']}
        elif name == 'folder.open.cached_rows' and folder in pending:
            pending[folder].setdefault('scan_started', event['unix_ms'])
        elif name == 'folder.open.ready' and folder in pending:
            times = pending.pop(folder)
            total = event['unix_ms'] - times['request']
            wait = times.get('scan_started', times['request']) - times['request']
            note = f'  (scan waited {wait} ms)' if wait > 100 else ''
            label = Path(folder).name if folder else '?'
            print(f'  {label:<24} {field(event, "image_count") or 0:>6} images {total:>6} ms{note}')

    if named('mainloop.probe'):
        # Startup (window construction and the first folder) is reported
        # separately from browsing, which the acceptance criteria refer to.
        first_ready = ready[0]['unix_ms'] if ready else float('inf')
        for label, keep in (('during startup', lambda t: t < first_ready),
                            ('while browsing', lambda t: t >= first_ready)):
            stalls = [field(e, 'late_ms') for e in named('mainloop.stall') if keep(e['unix_ms'])]
            over_100 = sum(1 for s in stalls if s >= 100)
            print(f'main-loop stalls {label} (>= 50 ms late): {len(stalls)}, '
                  f'over 100 ms: {over_100}, max: {max(stalls) if stalls else 0} ms')
    else:
        print('main-loop stalls: not recorded (log predates the stall probe)')

    worker = [field(e, 'worker_ms') for e in named('thumbnail.apply') if field(e, 'worker_ms') is not None]
    if worker:
        print(f'thumbnails applied: {len(worker)}; worker ms p50 {quantile(worker, .5)}, '
              f'p95 {quantile(worker, .95)}, max {max(worker)}')

    samples = named('process.sample')
    rss = [field(e, 'rss_kb') for e in samples if field(e, 'rss_kb')]
    cpu = [field(e, 'cpu_percent') for e in samples if field(e, 'cpu_percent') is not None]
    if rss:
        print(f'memory RSS MB: start {rss[0] // 1024}, peak {max(rss) // 1024}, end {rss[-1] // 1024}')
    if cpu:
        print(f'CPU %: median {quantile(cpu, .5):.0f}, peak {max(cpu):.0f}')


if __name__ == '__main__':
    main()
