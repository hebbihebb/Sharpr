#!/usr/bin/env python3
"""Score image-search benchmark reports against human relevance judgments.

Usage:
    python3 tools/benchmark-score.py --judgments search-judgments.json --report A.json [--report B.json]

Applies the ROADMAP Phase 3 rubric: a query passes when its top 20 hold at
least 10 relevant images, at least one of them in the top 5. A candidate
passes when at least 80% of queries pass (24 of 30) and every category
passes too. Unjudged queries fail.
"""
import argparse
import json
from collections import defaultdict
from pathlib import Path

MIN_RELEVANT = 10
TOP_FIVE = 5
PASS_SHARE = 0.8


def score(report, judgments):
    per_category = defaultdict(lambda: [0, 0])
    passed = 0
    rows = []
    for result in report['results']:
        judged = judgments.get(result['text'], {})
        relevant = set(judged.get('relevant', [])) if judged.get('judged') else set()
        hits = [path in relevant for path in result['top20']]
        ok = bool(judged.get('judged')) and sum(hits) >= MIN_RELEVANT and any(hits[:TOP_FIVE])
        category = judged.get('category', 'unjudged')
        per_category[category][1] += 1
        if ok:
            per_category[category][0] += 1
            passed += 1
        rows.append((result['text'], sum(hits), any(hits[:TOP_FIVE]), bool(judged.get('judged')), ok))
    total = len(report['results'])
    categories_pass = all(p >= PASS_SHARE * n for p, n in per_category.values())
    return {
        'passed': passed,
        'total': total,
        'gate': passed >= PASS_SHARE * total and categories_pass,
        'categories': {c: f'{p}/{n}' for c, (p, n) in sorted(per_category.items())},
        'rows': rows,
    }


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--judgments', type=Path, required=True)
    p.add_argument('--report', type=Path, action='append', required=True)
    p.add_argument('--detail', action='store_true', help='print every query')
    args = p.parse_args()
    judgments = json.loads(args.judgments.read_text())['queries']
    for path in args.report:
        result = score(json.loads(path.read_text()), judgments)
        verdict = 'PASSES' if result['gate'] else 'does not pass'
        print(f"{path.name}: {result['passed']}/{result['total']} queries pass; gate {verdict}")
        for category, share in result['categories'].items():
            print(f'  {category:<15} {share}')
        if args.detail:
            for text, relevant, top5, judged, ok in result['rows']:
                mark = 'pass' if ok else ('unjudged' if not judged else 'fail')
                print(f'    {mark:<8} {relevant:>2}/20 relevant  top5={"yes" if top5 else "no "}  {text}')


if __name__ == '__main__':
    main()
