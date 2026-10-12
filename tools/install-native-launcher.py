#!/usr/bin/env python3
"""Install the native development launcher using this checkout's built binary."""
from pathlib import Path
import shutil
import subprocess

root = Path(__file__).resolve().parents[1]
app = root / 'sharpr'
binary = app / 'target/debug/sharpr'
if not binary.is_file():
    raise SystemExit('Build sharpr with cargo build before installing the launcher')
subprocess.run(['glib-compile-schemas', str(app / 'data')], check=True)
applications = Path.home() / '.local/share/applications'
applications.mkdir(parents=True, exist_ok=True)
icon_dir = Path.home() / '.local/share/icons/hicolor/256x256/apps'
icon_dir.mkdir(parents=True, exist_ok=True)
shutil.copy2(app / 'data/io.github.hebbihebb.Sharpr.png', icon_dir / 'io.github.hebbihebb.Sharpr.png')
def desktop_quote(value):
    return '"' + str(value).replace('\\', '\\\\').replace('"', '\\"').replace('`', '\\`').replace('$', '\\$') + '"'
# Keep the existing filename so an upgrade does not leave a second launcher.
(applications / 'sharpr-dev.desktop').write_text(
    '[Desktop Entry]\nName=Sharpr\nGenericName=Image Viewer\n'
    'Comment=Review and curate images\n'
    f'Exec=env {desktop_quote("GSETTINGS_SCHEMA_DIR=" + str(app / "data"))} {desktop_quote(binary)} %F\n'
    f'TryExec={binary}\nPath={app}\n'
    'Icon=io.github.hebbihebb.Sharpr\nTerminal=false\nType=Application\n'
    'Categories=Graphics;Viewer;GNOME;GTK;\n'
    'MimeType=image/jpeg;image/png;image/gif;image/webp;image/tiff;image/bmp;\n'
    'StartupWMClass=sharpr\nKeywords=images;photos;viewer;wallpapers;\n'
)
if shutil.which('update-desktop-database'):
    subprocess.run(['update-desktop-database', str(applications)], check=True)
