#!/usr/bin/env python3
"""Compare two image-search benchmark runs of the same model (ROADMAP Phase 3).

Usage:
    python3 tools/benchmark-parity.py --manifest M.json --reference REF.json --native NAT.json

Reads each report and its saved `.vectors.npy`. Reports per-image cosine
agreement of the image vectors and per-query overlap of the top-20 lists.
Development tool only; it reads private files under .tmp/ and writes nothing.
"""
import argparse
import json
from pathlib import Path


def embedded_paths(manifest, report):
    """Rows of the vector matrix: manifest images minus failed ones, in order."""
    failed = {e['path'] for e in report.get('errors', [])}
    images = manifest['images'][:report['images'] + len(failed)]
    return [p for p in images if p not in failed]


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--manifest', type=Path, required=True)
    p.add_argument('--reference', type=Path, required=True)
    p.add_argument('--native', type=Path, required=True)
    args = p.parse_args()
    import numpy as np

    manifest = json.loads(args.manifest.read_text())
    reports = {name: json.loads(path.read_text()) for name, path in
               (('reference', args.reference), ('native', args.native))}
    vectors = {name: np.load(path.with_suffix('.vectors.npy')) for name, path in
               (('reference', args.reference), ('native', args.native))}
    rows = {name: embedded_paths(manifest, reports[name]) for name in reports}
    for name in reports:
        if len(rows[name]) != len(vectors[name]):
            raise SystemExit(f'{name}: {len(rows[name])} paths but {len(vectors[name])} vectors')
    if vectors['reference'].shape[1] != vectors['native'].shape[1]:
        raise SystemExit(f"dimension mismatch: {vectors['reference'].shape} vs {vectors['native'].shape}")

    index = {path: i for i, path in enumerate(rows['native'])}
    shared = [(i, index[path]) for i, path in enumerate(rows['reference']) if path in index]
    ref = vectors['reference'][[i for i, _ in shared]]
    nat = vectors['native'][[j for _, j in shared]]
    ref = ref / np.linalg.norm(ref, axis=1, keepdims=True)
    nat = nat / np.linalg.norm(nat, axis=1, keepdims=True)
    cosine = (ref * nat).sum(axis=1)

    overlaps = []
    native_results = {r['text']: r for r in reports['native']['results']}
    for result in reports['reference']['results']:
        other = native_results.get(result['text'])
        if other:
            overlaps.append(len(set(result['top20']) & set(other['top20'])) / 20)

    summary = {
        'images_compared': len(shared),
        'dimensions': int(ref.shape[1]),
        'cosine_median': float(np.median(cosine)),
        'cosine_p5': float(np.percentile(cosine, 5)),
        'cosine_min': float(cosine.min()),
        'queries_compared': len(overlaps),
        'top20_overlap_mean': float(np.mean(overlaps)) if overlaps else None,
        'top20_overlap_min': float(min(overlaps)) if overlaps else None,
    }
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()
