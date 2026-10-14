#!/usr/bin/env python3
"""Install EmbeddingGemma 2 and llama-server for Sharpr's visual search (D23).

Copies the two model files into ~/.local/share/sharpr-models/embeddinggemma-2/
and llama-server with its libraries into runtime/ there. Sharpr only reads
this folder and never downloads anything. Each file is copied to a partial
name, checked against its source, then renamed into place; files that are
already identical are left alone. Flatpak builds of Sharpr bring their own
llama-server and read the same model folder.

    python3 tools/install-visual-search-model.py --models DIR --runtime DIR
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
    args = parser.parse_args()

    sources = [args.models / name for name in MODEL_FILES]
    server = args.runtime / 'llama-server'
    for path in sources + [server]:
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
    libraries = sorted(p for p in args.runtime.iterdir() if p.name.startswith('lib') and '.so' in p.name)
    for path in [server] + libraries:
        changed += install(path, runtime / path.name)
    print(f'Visual search model ready in {folder} ({changed} files updated)')


if __name__ == '__main__':
    main()
