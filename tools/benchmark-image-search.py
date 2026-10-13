#!/usr/bin/env python3
"""Offline reference benchmark. It never downloads models or edits source images."""
import argparse
import json
import resource
import hashlib
import subprocess
import time
from pathlib import Path


def file_digest(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as handle:
        for block in iter(lambda: handle.read(1 << 20), b''):
            digest.update(block)
    return {'bytes': path.stat().st_size, 'sha256': digest.hexdigest()}


def provenance(model_dir, kind, dtype, torch):
    """What a later reader needs to reproduce this run (ROADMAP Phase 3)."""
    import importlib.metadata
    import platform
    libraries = {}
    for name in ('torch', 'transformers', 'sentence-transformers', 'pillow', 'numpy'):
        try:
            libraries[name] = importlib.metadata.version(name)
        except importlib.metadata.PackageNotFoundError:
            pass
    return {
        'python': platform.python_version(),
        'platform': platform.platform(),
        'libraries': libraries,
        'threads': torch.get_num_threads(),
        'dtype': str(dtype),
        'model_files': {str(f.relative_to(model_dir)): file_digest(f)
                        for f in sorted(model_dir.rglob('*')) if f.is_file()},
        'preprocessing': 'EXIF transpose, RGB, thumbnail to 768x768; JPEG XL via ImageMagick '
                         '-auto-orient -thumbnail 768x768',
        'query_prompt': "prompt_name='SearchQuery'" if kind == 'gemma' else "padding='max_length'",
    }


def main():
    p = argparse.ArgumentParser(__doc__)
    p.add_argument('--model-dir', type=Path, required=True)
    p.add_argument('--kind', choices=('gemma', 'siglip'), required=True)
    p.add_argument('--manifest', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--limit', type=int)
    args = p.parse_args()
    manifest = json.loads(args.manifest.read_text())
    if args.limit:
        manifest['images'] = manifest['images'][:args.limit]
    import numpy as np
    import torch
    from PIL import Image, ImageOps
    torch.set_num_threads(2)
    dtype = torch.bfloat16 if torch.cpu._is_avx512_bf16_supported() else torch.float32
    if args.kind == 'gemma':
        from sentence_transformers import SentenceTransformer
        model = SentenceTransformer(str(args.model_dir), local_files_only=True,
            config_kwargs={'audio_config': None}, model_kwargs={'torch_dtype': dtype}, device='cpu')
        def embed_image(image):
            return model.encode({'image': image}, normalize_embeddings=True)
        def embed_query(text):
            return model.encode(text, prompt_name='SearchQuery', normalize_embeddings=True)
    else:
        from transformers import AutoModel, AutoProcessor
        model = AutoModel.from_pretrained(str(args.model_dir), local_files_only=True).eval()
        processor = AutoProcessor.from_pretrained(str(args.model_dir), local_files_only=True)
        def normalized(tensor):
            if hasattr(tensor, 'pooler_output'):
                tensor = tensor.pooler_output
            return torch.nn.functional.normalize(tensor, dim=-1)[0].detach().float().numpy()
        def embed_image(image):
            with torch.inference_mode():
                return normalized(model.get_image_features(**processor(images=image, return_tensors='pt')))
        def embed_query(text):
            with torch.inference_mode():
                return normalized(model.get_text_features(**processor(text=[text], padding='max_length', return_tensors='pt')))
    paths, vectors, errors = [], [], []
    start = time.perf_counter()
    cache = args.output.parent / 'prepared-images'
    cache.mkdir(parents=True, exist_ok=True)
    for path in manifest['images']:
        try:
            decode_path = path
            if Path(path).suffix.lower() == '.jxl':
                stat = Path(path).stat()  # a replaced file must not reuse a stale preview
                key = f'{path}|{stat.st_size}|{stat.st_mtime_ns}'
                prepared = cache / (hashlib.sha256(key.encode()).hexdigest() + '.png')
                if not prepared.exists():
                    subprocess.run(['magick', path, '-auto-orient', '-thumbnail', '768x768', str(prepared)], check=True, capture_output=True, timeout=60)
                decode_path = prepared
            with Image.open(decode_path) as image:
                image = ImageOps.exif_transpose(image).convert('RGB')
                image.thumbnail((768, 768))
                vector = np.asarray(embed_image(image), dtype=np.float32).reshape(-1)
                if not np.isfinite(vector).all():
                    raise ValueError('Non-finite image vector')
                paths.append(path)
                vectors.append(vector)
        except Exception as error:
            errors.append({'path': path, 'error': str(error)})
        if len(paths) <= 3 or len(paths) % 50 == 0:
            print(json.dumps({'indexed': len(paths), 'errors': len(errors), 'seconds': time.perf_counter()-start}), flush=True)
    matrix = np.asarray(vectors, dtype=np.float32)
    if len(matrix) == 0:
        raise SystemExit('No images embedded; benchmark failed')
    seconds = time.perf_counter() - start
    results, latencies = [], []
    for item in manifest['queries']:
        embed_query(item['text'])  # explicit warmup
        start = time.perf_counter()
        query = embed_query(item['text'])
        scores = matrix @ query
        order = np.argsort(-scores)[:20]
        latency = time.perf_counter() - start
        latencies.append(latency)
        matches = [paths[int(i)] for i in order]
        expected = item.get('expected', [])
        results.append({'text': item['text'], 'top20': matches, 'scores': [float(scores[i]) for i in order],
            'latency_seconds': latency, 'hit': bool(set(expected) & set(matches)) if expected else None})
    judged = [r for r in results if r['hit'] is not None]
    report = {'dtype': str(dtype) if args.kind == 'gemma' else 'float32', 'kind': args.kind, 'model_dir': str(args.model_dir.resolve()), 'images': len(paths),
        'index_seconds': seconds, 'images_per_second': len(paths)/seconds,
        'peak_rss_kib': resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
        'warm_p95_seconds': float(np.percentile(latencies, 95)),
        'hit_rate': sum(r['hit'] for r in judged)/len(judged) if judged else None,
        'gate': 'pending: need 1000 images, 30 judged queries, native parity and navigation measurements',
        'provenance': provenance(args.model_dir, args.kind, dtype, torch),
        'errors': errors, 'results': results}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2))
    np.save(args.output.with_suffix('.vectors.npy'), matrix)
    print(json.dumps({k:v for k,v in report.items() if k not in ('results','errors')}), flush=True)


if __name__ == '__main__':
    main()
