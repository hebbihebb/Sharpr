#!/usr/bin/env python3
"""The D48 real-detail gate (ROADMAP Work Order item 39).

Proves the detail estimator before any interface is built. It is read-only
on the library; everything it writes goes under the output folder
(default `.tmp/detail-gate/` in the repository, which git ignores).

Steps (each can run alone):
  candidates  find labelled fake-4K candidates without the estimator: 4K
              images with a clearly smaller version elsewhere in the
              library, plus file names that name an upscaler
  run         measure camera originals scaled to 4K and synthetic fakes
              made from them, the candidates and a library sample, through
              the app's own estimator (an ignored Rust test, release build)
  report      confusion tables per treatment and filter, coverage, the
              contact sheet of the lowest-detail sample images and the
              candidate sheet

The Rust side runs in a memory-capped scope, one heavy job at a time.
Library images never leave the output folder.
"""

import argparse
import html
import json
import os
import re
import statistics
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CRATE = REPO / "sharpr"
IMAGE_EXTENSIONS = {".jpg", ".jpeg", ".png", ".webp", ".jxl", ".tif", ".tiff", ".bmp", ".avif", ".heic", ".heif", ".gif"}
UPSCALER_NAMES = re.compile(r"(upscal|[^a-z0-9]x[248][^0-9]|[^a-z0-9][248]x[^a-z0-9]|waifu2x|esrgan|seedvr|topaz|gigapixel|realesr|supir)", re.I)


def cargo_test(name, env):
    """Runs one ignored Rust test in a release build without fat LTO."""
    env = {**os.environ, **env,
           "CARGO_PROFILE_RELEASE_LTO": "off",
           "CARGO_PROFILE_RELEASE_CODEGEN_UNITS": "16"}
    # A scope keeps the caller's environment.
    command = ["systemd-run", "--user", "--scope", "--quiet", "-p", "MemoryMax=9G", "-p", "MemorySwapMax=0",
               "cargo", "test", "--release", "--bin", "sharpr", name, "--", "--ignored", "--exact", "--nocapture"]
    subprocess.run(command, cwd=CRATE, env=env, check=True)


def image_size(path):
    from PIL import Image  # only for sizes, EXIF and crops; never written back
    with Image.open(path) as image:
        return image.size


def camera_originals(folders, limit):
    """Camera images (EXIF Make present) with a short side of at least 2900
    px, so scaling them to 4K (2160) removes the softness of the camera's
    own pixel grid and leaves genuinely full detail. Spread evenly over the
    folders' images in path order, so no one shoot dominates."""
    from PIL import Image
    Image.MAX_IMAGE_PIXELS = None  # sizes only; nothing is decoded here
    found = []
    for folder in folders:
        for path in sorted(Path(folder).rglob("*")):
            if path.suffix.lower() not in {".jpg", ".jpeg"}:
                continue
            try:
                with Image.open(path) as image:
                    if min(image.size) >= 2900 and image.getexif().get(0x010F):
                        found.append(str(path))
            except OSError:
                continue
    if len(found) <= limit:
        return found
    step = len(found) / limit
    return [found[int(i * step)] for i in range(limit)]


def library_images(root):
    return sorted(str(p) for p in Path(root).rglob("*")
                  if p.is_file() and p.suffix.lower() in IMAGE_EXTENSIONS
                  and not any(part.startswith(".") for part in p.relative_to(root).parts))


def step_candidates(args):
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    found = out / "candidates-vectors.json"
    cargo_test("detail::tests::detail_labelled_candidates", {"SHARPR_DETAIL_CANDIDATES": str(found)})
    by_vectors = json.loads(found.read_text())
    # Wallpapers first (what the user sees), then the other folders; one
    # entry per picture name.
    def folder_rank(path):
        rel = os.path.relpath(path, args.library)
        return next((i for i, name in enumerate(("Wallpapers", "Needs Upscale", "Needs Review")) if rel.startswith(name)), 9)
    by_vectors.sort(key=lambda c: (folder_rank(c["path"]), -c["similarity"], c["path"]))
    chosen, seen, names = [], set(), set()
    for c in by_vectors:
        name = Path(c["path"]).stem
        if c["path"] in seen or name in names:
            continue
        seen.add(c["path"])
        names.add(name)
        chosen.append({**c, "evidence": "smaller version elsewhere: %s (%d×%d)" % (
            c["source"], *c["source_size"])})
    for path in library_images(args.library):
        if path in seen or not UPSCALER_NAMES.search(Path(path).name):
            continue
        try:
            w, h = image_size(path)
        except Exception:
            continue
        if max(w, h) >= 3840 and min(w, h) >= 2160:
            chosen.append({"path": path, "size": [w, h], "evidence": "file name names an upscaler"})
            seen.add(path)
    (out / "candidates.json").write_text(json.dumps(chosen, indent=1))
    print(f"{len(by_vectors)} images have a smaller version; {len(chosen)} candidates in total")


def step_run(args):
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    candidates = json.loads((out / "candidates.json").read_text()) if (out / "candidates.json").exists() else []
    manifest = {
        "natives": [] if args.no_natives else camera_originals(args.natives.split(","), args.native_count),
        "labelled": [c["path"] for c in candidates[: args.labelled_count]],
        "sample": [] if args.no_sample else library_images(args.sample),
    }
    (out / "manifest.json").write_text(json.dumps(manifest, indent=1))
    print(f"{len(manifest['natives'])} camera originals, {len(manifest['labelled'])} labelled candidates, "
          f"{len(manifest['sample'])} sample images")
    cargo_test("detail::tests::detail_gate_report", {
        "SHARPR_DETAIL_MANIFEST": str(out / "manifest.json"),
        "SHARPR_DETAIL_OUTPUT": str(out / args.results.split(",")[-1]),
    })


def pct(n, d):
    return f"{100 * n / d:.0f}% ({n}/{d})" if d else "n/a"


def step_report(args):
    out = Path(args.out)
    rows = [json.loads(line) for name in args.results.split(",")
            for line in (out / name).read_text().splitlines() if line.strip()]
    # Natives and fakes are judged from the losses recorded at the chosen
    # probe, with the chosen share: the same arithmetic as the estimator.
    import math
    ladder_all = [540, 720, 1080, 1440, 2160, 2880, 4320, 5760, 8640, 11520]

    def evaluate(r):
        losses = r.get("probes", {}).get(args.probe)
        if losses is None:
            return r
        short_side = min(r["width"], r["height"])
        span = math.log(short_side / ladder_all[0])
        estimate = "full"
        for level, median, _ in losses:
            share = math.log(short_side / (level * float(args.probe))) / span
            if median <= args.k * share:
                estimate = f"below:{level}"
                break
        if not losses:
            estimate = "too-small"
        return {**r, "estimate": estimate, "upscaled": estimate.startswith("below:")}

    rows = [evaluate(r) if r["set"] in ("native", "fake") else r for r in rows]
    natives = [r for r in rows if r["set"] == "native" and "error" not in r]
    fakes = [r for r in rows if r["set"] == "fake"]
    lines = ["# D48 gate results", "",
             f"Natives and fakes evaluated at probe {args.probe}, near-lossless share {args.k}; "
             "labelled and sample images measured by the shipped estimator.", ""]

    def short(r):
        return min(r["width"], r["height"])

    def flagged(r):
        return r.get("upscaled", False)

    def within_step(r):
        # Within one ladder size of the true source.
        ladder = [540, 720, 1080, 1440, 2160, 2880, 4320]
        if not r["estimate"].startswith("below:"):
            return False
        est = int(r["estimate"].split(":")[1])
        return abs(ladder.index(est) - ladder.index(r["source"])) <= 1 if est in ladder else False

    measurable_natives = [r for r in natives if not r["estimate"] in ("little-texture", "too-small")]
    lines += [f"Camera originals measured: {len(natives)} (not measurable: {len(natives) - len(measurable_natives)})",
              f"Natives falsely flagged: {pct(sum(map(flagged, natives)), len(natives))}",
              f"Fakes flagged: {pct(sum(map(flagged, fakes)), len(fakes))}",
              f"Fakes within one ladder step of the source: {pct(sum(map(within_step, fakes)), len(fakes))}", ""]
    lines += ["| source | filter | treatment | flagged | within one step |", "| --- | --- | --- | --- | --- |"]
    groups = defaultdict(list)
    for r in fakes:
        groups[(r["source"], r["filter"], r["treatment"])].append(r)
    for key in sorted(groups):
        g = groups[key]
        lines.append(f"| {key[0]}p | {key[1]} | {key[2]} | {pct(sum(map(flagged, g)), len(g))} | "
                     f"{pct(sum(map(within_step, g)), len(g))} |")
    for name in ("treatment", "filter", "source"):
        lines += ["", f"By {name}:"]
        by = defaultdict(list)
        for r in fakes:
            by[r[name]].append(r)
        for k in sorted(by):
            lines.append(f"- {k}: flagged {pct(sum(map(flagged, by[k])), len(by[k]))}, "
                         f"within one step {pct(sum(map(within_step, by[k])), len(by[k]))}")
    estimates = defaultdict(int)
    for r in natives:
        estimates[r["estimate"]] += 1
    lines += ["", "Native estimates: " + ", ".join(f"{k} {v}" for k, v in sorted(estimates.items()))]

    labelled = [r for r in rows if r["set"] == "labelled"]
    lines += ["", f"Labelled candidates (unconfirmed): {len(labelled)}, looks upscaled: "
              f"{sum(1 for r in labelled if r.get('upscaled'))}"]
    sample = [r for r in rows if r["set"] == "sample"]
    if sample:
        ok = [r for r in sample if "error" not in r]
        seconds = [r["seconds"] for r in ok]
        lines += ["", f"Sample ({args.sample}): {len(sample)} files, {len(sample) - len(ok)} undecodable, "
                  f"{sum(1 for r in ok if r['upscaled'])} look upscaled, "
                  f"{sum(1 for r in ok if r['estimate'] in ('little-texture', 'too-small'))} not measurable",
                  f"Time per image: median {statistics.median(seconds):.2f} s, mean {statistics.mean(seconds):.2f} s "
                  "(decode and measure, one at a time)" if seconds else ""]
        total = len(library_images(args.library))
        lines.append(f"Library under {args.library}: {total} image files; a full pass at this mean is about "
                     f"{total * statistics.mean(seconds) / 60:.0f} minutes at full speed (the background pass runs at idle priority)" if seconds else "")
        write_sheet(out, "contact-sheet.html", "Lowest real detail in the sample",
                    sorted(ok, key=detail_order)[:50])
    if labelled:
        candidates = {c["path"]: c for c in json.loads((out / "candidates.json").read_text())}
        for r in labelled:
            r["evidence"] = candidates.get(r["path"], {}).get("evidence", "")
        write_sheet(out, "labelled-candidates.html", "Labelled fake-4K candidates (please confirm)", labelled)
    (out / "report.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))


def detail_order(r):
    """Lowest detail first, relative to the pixel size; unmeasurable last."""
    d = r.get("detail")
    if not d:
        return (2, 0)
    return (0 if r["upscaled"] else 1, min(d) / max(1, min(r["width"], r["height"])), r["path"])


def write_sheet(out, name, title, rows):
    """An HTML sheet with each image's size, estimate, words and a 100% crop
    of its most textured window (written beside the sheet)."""
    from PIL import Image, ImageOps
    crops = out / "crops"
    crops.mkdir(exist_ok=True)
    cards = []
    for i, r in enumerate(rows):
        crop_html = ""
        if r.get("focus"):
            try:
                with Image.open(r["path"]) as image:
                    image = ImageOps.exif_transpose(image)
                    x, y = r["focus"]
                    crop = image.crop((x, y, x + 512, y + 512)).convert("RGB")
                    crop_path = crops / f"{name.split('.')[0]}-{i:03}.png"
                    crop.save(crop_path)
                    crop_html = f'<img src="crops/{crop_path.name}" width="512" height="512">'
            except Exception:
                # JPEG XL and others PIL can't open: ImageMagick crops them
                # (auto-orient first, as the estimator sees them upright).
                crop_path = crops / f"{name.split('.')[0]}-{i:03}.png"
                x, y = r["focus"]
                done = subprocess.run(
                    ["magick", r["path"], "-auto-orient", "-crop", f"512x512+{x}+{y}", "+repage", str(crop_path)],
                    capture_output=True)
                crop_html = (f'<img src="crops/{crop_path.name}" width="512" height="512">'
                             if done.returncode == 0 else
                             f"<p>(no crop: {html.escape(done.stderr.decode(errors='replace')[:200])})</p>")
        losses = ", ".join(f"{l}p {m:.2f}≤{a:.2f}" for l, m, a in r.get("losses", []))
        cards.append(f"""<div class="card">{crop_html}
<p><b>{html.escape(Path(r['path']).name)}</b><br>{r['width']}×{r['height']} · {html.escape(r['words'])}
<br><small>{html.escape(r.get('evidence', ''))}</small>
<br><small>{r['tiles']} textured tiles · {losses}</small>
<br><small>{html.escape(r['path'])}</small></p></div>""")
    (out / name).write_text(f"""<!doctype html><meta charset="utf-8"><title>{html.escape(title)}</title>
<style>body{{background:#1b1b1b;color:#ddd;font:13px monospace}}.card{{display:inline-block;vertical-align:top;
width:520px;margin:8px}}img{{image-rendering:pixelated;background:#000}}</style>
<h1>{html.escape(title)}</h1><p>Crops are at 100%, from the most textured part of each image.</p>
{''.join(cards)}""")


def main():
    home = Path.home()
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("steps", nargs="+", choices=["candidates", "run", "report"])
    parser.add_argument("--out", default=str(REPO / ".tmp" / "detail-gate"))
    parser.add_argument("--library", default=str(home / "Pictures"))
    parser.add_argument("--natives", default=f"{home}/Pictures/Photo,{home}/Pictures/ARW",
                        help="comma-separated folders of camera images")
    parser.add_argument("--native-count", type=int, default=40)
    parser.add_argument("--labelled-count", type=int, default=10)
    parser.add_argument("--sample", default=str(home / "Pictures" / "Wallpapers"))
    parser.add_argument("--no-sample", action="store_true", help="skip the library sample (faster tuning)")
    parser.add_argument("--no-natives", action="store_true", help="skip natives and fakes (labelled and sample only)")
    parser.add_argument("--results", default="results.jsonl",
                        help="result files in the output folder, comma-separated (natives/fakes and sample may come from separate runs)")
    parser.add_argument("--probe", default="1.4", help="probe factor to evaluate natives and fakes at, from recorded losses")
    parser.add_argument("--k", type=float, default=0.25, help="near-lossless share to evaluate at (detail::NEAR_LOSSLESS)")
    args = parser.parse_args()
    for step in args.steps:
        {"candidates": step_candidates, "run": step_run, "report": step_report}[step](args)


if __name__ == "__main__":
    sys.exit(main())
