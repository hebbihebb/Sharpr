#!/usr/bin/env python3
"""Score image-search benchmark reports against human relevance judgments.

Usage:
    python3 tools/benchmark-score.py --manifest M.json --judgments search-judgments.json \\
        --report A.json [--report B.json]

Applies the ROADMAP Phase 3 rubric: a query passes when its top 20 hold at
least 10 relevant images, at least one of them in the top 5. A candidate
passes when at least 80% of the manifest's queries pass (24 of 30) and each of
the four categories has queries and passes too. Unjudged queries, and queries
missing from a report, fail and still count in the denominator.
"""
import argparse
import json
from collections import defaultdict
from pathlib import Path

MIN_RELEVANT = 10
TOP_FIVE = 5
PASS_SHARE = 0.8
# Keep in sync with CATEGORIES in tools/benchmark-judge.py.
CATEGORIES = ['subject', 'style', 'mood & colour', 'people']


def score(queries, report, judgments):
    """Score `report` over every query text in `queries` (the manifest order)."""
    results = {r['text']: r for r in report['results']}
    per_category = defaultdict(lambda: [0, 0], {c: [0, 0] for c in CATEGORIES})
    passed = 0
    rows = []
    for text in queries:
        judged = judgments.get(text, {})
        is_judged = bool(judged.get('judged'))
        relevant = set(judged.get('relevant', [])) if is_judged else set()
        result = results.get(text)
        top20 = result['top20'][:20] if result else []
        hits = [path in relevant for path in top20]
        ok = is_judged and result is not None and sum(hits) >= MIN_RELEVANT and any(hits[:TOP_FIVE])
        category = judged.get('category') if is_judged else 'unjudged'
        per_category[category][1] += 1
        if ok:
            per_category[category][0] += 1
            passed += 1
        status = 'pass' if ok else ('missing' if result is None else ('unjudged' if not is_judged else 'fail'))
        rows.append((text, sum(hits), any(hits[:TOP_FIVE]), status))
    total = len(queries)
    categories_pass = all(n > 0 and p >= PASS_SHARE * n for p, n in per_category.values())
    return {
        'passed': passed,
        'total': total,
        'gate': total > 0 and passed >= PASS_SHARE * total and categories_pass,
        'categories': {c: f'{p}/{n}' for c, (p, n) in per_category.items()},
        'extra': sorted(set(results) - set(queries)),
        'rows': rows,
    }


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--manifest', type=Path, required=True, help='the benchmark manifest with every query')
    p.add_argument('--judgments', type=Path, required=True)
    p.add_argument('--report', type=Path, action='append', required=True)
    p.add_argument('--detail', action='store_true', help='print every query')
    args = p.parse_args()
    queries = [q['text'] for q in json.loads(args.manifest.read_text())['queries']]
    if not queries:
        raise SystemExit('the manifest has no queries')
    judgments = json.loads(args.judgments.read_text())['queries']
    for path in args.report:
        result = score(queries, json.loads(path.read_text()), judgments)
        verdict = 'PASSES' if result['gate'] else 'does not pass'
        print(f"{path.name}: {result['passed']}/{result['total']} queries pass; gate {verdict}")
        for category, share in result['categories'].items():
            note = '  (no queries: fails)' if share.endswith('/0') else ''
            print(f'  {category:<15} {share}{note}')
        if result['extra']:
            print(f"  ignored {len(result['extra'])} report queries not in the manifest")
        if args.detail:
            for text, relevant, top5, status in result['rows']:
                print(f'    {status:<8} {relevant:>2}/20 relevant  top5={"yes" if top5 else "no "}  {text}')


if __name__ == '__main__':
    main()
