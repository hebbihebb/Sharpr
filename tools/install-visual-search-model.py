#!/usr/bin/env python3
"""Install EmbeddingGemma 2 and llama-server for Sharpr's visual search (D23).

Copies the two model files into ~/.local/share/sharpr-models/embeddinggemma-2/
and llama-server with its libraries into runtime/ there. With --gpu-runtime, a
GPU (Vulkan) build goes into runtime-gpu/; Sharpr then indexes on the graphics
card, about 17 times faster, and keeps the processor build for searches. Sharpr only reads
this folder and never downloads anything. Each file is copied to a partial
name, checked against its source, then renamed into place; files that are
already identical are left alone. Flatpak builds of Sharpr bring their own
llama-server and read the same model folder.

    python3 tools/install-visual-search-model.py --models DIR --runtime DIR [--gpu-runtime DIR]
"""
import argparse
import hashlib
import os
import shutil
from pathlib import Path

MODEL_FILES = ('embeddinggemma-2-Q8_0.gguf', 'mmproj-embeddinggemma-2-Q8_0.gguf')


def digest(path):
    hasher = hashlib.sha256()
    with open(path, 'rb') as handle:
        for block in iter(lambda: handle.read(1 << 20), b''):
            hasher.update(block)
    return hasher.hexdigest()


def install(source, dest):
    """Copies one file (or recreates one symlink) unless already identical."""
    if source.is_symlink():
        target = os.readlink(source)
        if dest.is_symlink() and os.readlink(dest) == target:
            return False
        partial = dest.with_name(dest.name + '.partial')
        if partial.is_symlink() or partial.exists():
            partial.unlink()
        partial.symlink_to(target)
        os.replace(partial, dest)
        return True
    if dest.is_file() and not dest.is_symlink() and dest.stat().st_size == source.stat().st_size \
            and digest(dest) == digest(source):
        return False
    partial = dest.with_name(dest.name + '.partial')
    shutil.copy2(source, partial)
    if digest(partial) != digest(source):
        partial.unlink()
        raise SystemExit(f'Copy of {source} does not match; nothing replaced')
    os.replace(partial, dest)
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--models', type=Path, required=True,
                        help='folder holding ' + ' and '.join(MODEL_FILES))
    parser.add_argument('--runtime', type=Path, required=True,
                        help='folder holding llama-server and its lib*.so* files')
    parser.add_argument('--gpu-runtime', type=Path,
                        help='folder holding a GPU (Vulkan) llama-server and its libraries')
    args = parser.parse_args()

    sources = [args.models / name for name in MODEL_FILES]
    server = args.runtime / 'llama-server'
    gpu_server = args.gpu_runtime / 'llama-server' if args.gpu_runtime else None
    for path in sources + [server] + ([gpu_server] if gpu_server else []):
        if not path.is_file():
            raise SystemExit(f'Missing {path}')
    for path in sources:
        with open(path, 'rb') as handle:
            if handle.read(4) != b'GGUF':
                raise SystemExit(f'{path} is not a GGUF model file')

    data = Path(os.environ.get('XDG_DATA_HOME') or Path.home() / '.local/share')
    folder = data / 'sharpr-models/embeddinggemma-2'
    runtime = folder / 'runtime'
    runtime.mkdir(parents=True, exist_ok=True)
    changed = sum(install(path, folder / path.name) for path in sources)
    for source_dir, dest_dir in [(args.runtime, runtime)] + (
            [(args.gpu_runtime, folder / 'runtime-gpu')] if args.gpu_runtime else []):
        dest_dir.mkdir(exist_ok=True)
        libraries = sorted(p for p in source_dir.iterdir() if p.name.startswith('lib') and '.so' in p.name)
        for path in [source_dir / 'llama-server'] + libraries:
            changed += install(path, dest_dir / path.name)
    print(f'Visual search model ready in {folder} ({changed} files updated)')


if __name__ == '__main__':
    main()
