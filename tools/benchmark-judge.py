#!/usr/bin/env python3
"""Build a blind relevance-judging sheet for the image-search benchmark.

Usage:
    python3 tools/benchmark-judge.py --manifest M.json --report A.json --report B.json \\
        --output .tmp/judge.html

For each query the top-20 results of every report are pooled, de-duplicated
and shuffled, so the judge cannot tell which model found an image. Open the
HTML file in a browser, click the relevant images, mark each query as judged,
pick its category, then press "Save judgments" to download a JSON file for
tools/benchmark-score.py. Progress is kept in the browser between visits.

Previews come from the GNOME thumbnail cache when it is fresh for the current
file, or are generated once into --previews, keyed by path, size and mtime (needs
Pillow, e.g. the .tmp/search-benchmark environment). Everything stays local; keep
the output under .tmp/ because it lists private image paths.
"""
import argparse
import hashlib
import json
import random
import struct
import subprocess
from pathlib import Path
from urllib.parse import quote

CATEGORIES = ['subject', 'style', 'mood & colour', 'people']


def guess_category(text):
    lower = text.lower()
    if any(w in lower for w in ('anime', 'watercolor', 'black and white', 'drawing')):
        return 'style'
    if any(w in lower for w in ('girl', 'woman', 'group', 'people', 'bikini', 'muscles')):
        return 'people'
    if any(w in lower for w in ('dark', 'black', 'sky', 'storm', 'blue')):
        return 'mood & colour'
    return 'subject'


def png_text(path):
    """The tEXt chunks of a PNG file as a dict (stops at the image data)."""
    chunks = {}
    with open(path, 'rb') as f:
        if f.read(8) != b'\x89PNG\r\n\x1a\n':
            return chunks
        while header := f.read(8):
            if len(header) < 8:
                break
            length, kind = struct.unpack('>I4s', header)
            if kind in (b'IDAT', b'IEND'):
                break
            body = f.read(length)
            f.seek(4, 1)
            if kind == b'tEXt' and b'\0' in body:
                key, value = body.split(b'\0', 1)
                chunks[key.decode('latin-1')] = value.decode('latin-1')
    return chunks


def fresh_thumbnail(thumb, stat):
    """A GNOME thumbnail is valid only if it records the source's current mtime
    (and size, when present), as the freedesktop thumbnail spec requires."""
    try:
        text = png_text(thumb)
    except OSError:
        return False
    if text.get('Thumb::MTime') != str(int(stat.st_mtime)):
        return False
    return 'Thumb::Size' not in text or text['Thumb::Size'] == str(stat.st_size)


def preview(path, previews):
    """A small browser-friendly preview for `path`: a fresh cached GNOME
    thumbnail, or a 320 px JPEG generated into `previews` (JPEG XL via
    ImageMagick). Previews always show the file's current pixels."""
    stat = Path(path).stat()
    uri = 'file://' + quote(str(Path(path).resolve()))
    digest = hashlib.md5(uri.encode()).hexdigest()
    for size in ('large', 'x-large', 'normal'):
        thumb = Path.home() / '.cache/thumbnails' / size / f'{digest}.png'
        if thumb.exists() and fresh_thumbnail(thumb, stat):
            return thumb.as_uri()
    key = f'{path}\0{stat.st_size}\0{stat.st_mtime_ns}'
    target = previews / f'{hashlib.sha256(key.encode()).hexdigest()}.jpg'
    if not target.exists():
        previews.mkdir(parents=True, exist_ok=True)
        if Path(path).suffix.lower() == '.jxl':
            subprocess.run(['magick', str(path), '-auto-orient', '-thumbnail', '320x320',
                            str(target)], check=True, capture_output=True, timeout=60)
        else:
            from PIL import Image, ImageOps
            with Image.open(path) as image:
                image = ImageOps.exif_transpose(image).convert('RGB')
                image.thumbnail((320, 320))
                image.save(target, quality=85)
    return target.resolve().as_uri()


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--manifest', type=Path, required=True)
    p.add_argument('--report', type=Path, action='append', required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--previews', type=Path, default=Path('.tmp/judge-previews'),
                   help='cache for generated previews (default .tmp/judge-previews)')
    args = p.parse_args()
    manifest = json.loads(args.manifest.read_text())
    reports = [json.loads(path.read_text()) for path in args.report]

    queries = []
    for item in manifest['queries']:
        text = item['text']
        pool = []
        for report in reports:
            result = next((r for r in report['results'] if r['text'] == text), None)
            for path in (result or {}).get('top20', []):
                if path not in pool:
                    pool.append(path)
        random.Random(f"{manifest.get('seed', 0)}:{text}").shuffle(pool)
        queries.append({
            'text': text,
            'category': guess_category(text),
            'images': [{'path': p, 'src': preview(p, args.previews)} for p in pool],
        })

    data = json.dumps({'key': f"sharpr-judge-{manifest.get('seed', 0)}", 'categories': CATEGORIES,
                       'queries': queries})
    # Escape only for the raw-text <script> element; JSON.parse restores every
    # character, so queries and paths keep their exact identity.
    safe = data.replace('&', '\\u0026').replace('<', '\\u003c').replace('>', '\\u003e')
    page = PAGE.replace('__DATA__', safe)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(page)
    total = sum(len(q['images']) for q in queries)
    print(f'{args.output}: {len(queries)} queries, {total} pooled results')


PAGE = r'''<!doctype html>
<meta charset="utf-8">
<title>Sharpr search judging</title>
<style>
  body { font: 14px system-ui, sans-serif; background: #1e1e1e; color: #eee; margin: 0 16px 80px; }
  header { position: sticky; top: 0; background: #1e1e1e; padding: 12px 0; z-index: 1;
           display: flex; gap: 16px; align-items: center; border-bottom: 1px solid #444; }
  section { margin: 24px 0; }
  h2 { font-size: 18px; margin: 0 0 8px; display: flex; gap: 12px; align-items: center; }
  .grid { display: flex; flex-wrap: wrap; gap: 6px; }
  .grid img { height: 140px; border: 4px solid transparent; border-radius: 6px; cursor: pointer; }
  .grid img.relevant { border-color: #57e389; }
  .done h2 { color: #8ff0a4; }
  button { font: inherit; padding: 6px 14px; }
</style>
<header>
  <strong>Click every relevant image, then tick "Judged".</strong>
  <span id="progress"></span>
  <button id="save">Save judgments</button>
</header>
<main id="queries"></main>
<script id="data" type="application/json">__DATA__</script>
<script>
const data = JSON.parse(document.getElementById('data').textContent);
let state = {};
try { state = JSON.parse(localStorage.getItem(data.key) || '{}'); } catch (e) {}
const persist = () => { try { localStorage.setItem(data.key, JSON.stringify(state)); } catch (e) {} };
const entry = q => (state[q.text] ||= { category: q.category, judged: false, relevant: [] });
function progress() {
  const done = data.queries.filter(q => entry(q).judged).length;
  document.getElementById('progress').textContent = `${done} / ${data.queries.length} judged`;
}
for (const q of data.queries) {
  const e = entry(q);
  const section = document.createElement('section');
  const h = document.createElement('h2');
  h.textContent = q.text;
  const select = document.createElement('select');
  for (const c of data.categories) select.add(new Option(c, c, false, c === e.category));
  select.onchange = () => { e.category = select.value; persist(); };
  const judged = document.createElement('label');
  const box = document.createElement('input');
  box.type = 'checkbox'; box.checked = e.judged;
  box.onchange = () => { e.judged = box.checked; section.classList.toggle('done', e.judged); persist(); progress(); };
  judged.append(box, ' Judged');
  h.append(select, judged);
  const grid = document.createElement('div');
  grid.className = 'grid';
  for (const img of q.images) {
    const el = document.createElement('img');
    el.loading = 'lazy'; el.src = img.src; el.title = img.path;
    el.classList.toggle('relevant', e.relevant.includes(img.path));
    el.onclick = () => {
      const i = e.relevant.indexOf(img.path);
      if (i >= 0) e.relevant.splice(i, 1); else e.relevant.push(img.path);
      el.classList.toggle('relevant', i < 0); persist();
    };
    grid.append(el);
  }
  section.classList.toggle('done', e.judged);
  section.append(h, grid);
  document.getElementById('queries').append(section);
}
progress();
document.getElementById('save').onclick = () => {
  const blob = new Blob([JSON.stringify({ judged_at: new Date().toISOString(), queries: state }, null, 2)],
                        { type: 'application/json' });
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob); a.download = 'search-judgments.json'; a.click();
};
</script>
'''

if __name__ == '__main__':
    main()
