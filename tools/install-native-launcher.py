#!/usr/bin/env python3
"""Install a release build of the current commit as the Sharpr desktop launcher.

The build is copied to ~/.local/opt/sharpr/<commit>/ with its own compiled
GSettings schema, and ~/.local/opt/sharpr/current points at it. Later
`cargo build` runs in the checkout therefore never change the launched app.
Earlier installs are kept for rollback:

    ln -sfn ~/.local/opt/sharpr/<older-commit> ~/.local/opt/sharpr/current
"""
from pathlib import Path
import argparse
import datetime
import os
import shutil
import subprocess

# No options; this makes --help (or a typo) stop instead of installing.
argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter).parse_args()

root = Path(__file__).resolve().parents[1]
app = root / 'sharpr'
opt = Path.home() / '.local/opt/sharpr'


def git(*args):
    return subprocess.run(['git', '-C', str(root), *args], check=True,
                          capture_output=True, text=True).stdout.strip()


if git('status', '--porcelain'):
    raise SystemExit('Commit or stash changes first: the installed build must match a commit.')
commit = git('rev-parse', 'HEAD')
short = commit[:7]

subprocess.run(['cargo', 'build', '--release', '--locked'], cwd=app, check=True)
binary = app / 'target/release/sharpr'

dest = opt / short
staging = opt / f'.{short}.partial'
shutil.rmtree(staging, ignore_errors=True)
(staging / 'schemas').mkdir(parents=True)
shutil.copy2(binary, staging / 'sharpr')
shutil.copy2(app / 'data/io.github.hebbihebb.Sharpr.gschema.xml', staging / 'schemas')
subprocess.run(['glib-compile-schemas', str(staging / 'schemas')], check=True)
(staging / 'COMMIT').write_text(
    f'{commit}\n{git("log", "-1", "--format=%s")}\n'
    f'installed {datetime.datetime.now().isoformat(timespec="seconds")}\n')
if dest.exists():
    shutil.rmtree(dest)
staging.rename(dest)

link = opt / 'current'
tmp_link = opt / '.current.tmp'
if tmp_link.is_symlink():
    tmp_link.unlink()
tmp_link.symlink_to(short)
os.replace(tmp_link, link)

icon_dir = Path.home() / '.local/share/icons/hicolor/512x512/apps'
icon_dir.mkdir(parents=True, exist_ok=True)
shutil.copy2(app / 'data/io.github.hebbihebb.Sharpr.png', icon_dir / 'io.github.hebbihebb.Sharpr.png')


def desktop_quote(value):
    return '"' + str(value).replace('\\', '\\\\').replace('"', '\\"').replace('`', '\\`').replace('$', '\\$') + '"'


current = link
applications = Path.home() / '.local/share/applications'
applications.mkdir(parents=True, exist_ok=True)
# Keep the existing filename so an upgrade does not leave a second launcher.
(applications / 'sharpr-dev.desktop').write_text(
    '[Desktop Entry]\nName=Sharpr\nGenericName=Image Viewer\n'
    'Comment=Find, curate and prepare wallpaper-grade images\n'
    f'Exec=env {desktop_quote("GSETTINGS_SCHEMA_DIR=" + str(current / "schemas"))} '
    f'{desktop_quote(current / "sharpr")}\n'
    f'TryExec={current / "sharpr"}\n'
    'Icon=io.github.hebbihebb.Sharpr\nTerminal=false\nType=Application\n'
    'Categories=Graphics;Viewer;GNOME;GTK;\n'
    'StartupWMClass=io.github.hebbihebb.Sharpr\nKeywords=images;photos;viewer;wallpapers;\n'
)
if shutil.which('update-desktop-database'):
    subprocess.run(['update-desktop-database', str(applications)], check=True)
print(f'Installed {short} to {dest}; launcher now runs {current / "sharpr"}')
