#!/usr/bin/env python3
"""Run a GTK check on a private headless Wayland display and D-Bus session.

Example (from sharpr/): ../tools/gtk-offscreen.py -- cargo nextest run ...
Pass isolated XDG directories, GSETTINGS_SCHEMA_DIR and GTK fixture paths as
usual. No windows or portal dialogs can appear on the user's desktop.
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "--inner":
        runtime = Path(sys.argv[2])
        command = sys.argv[3:]
        env = os.environ.copy()
        env.update(XDG_RUNTIME_DIR=str(runtime), WAYLAND_DISPLAY="sharpr-test",
                   GDK_BACKEND="wayland", GSK_RENDERER="cairo",
                   GSETTINGS_BACKEND="memory", GTK_IM_MODULE="simple",
                   GTK_USE_PORTAL="0", LIBGL_ALWAYS_SOFTWARE="1")
        with (runtime / "mutter.log").open("w") as log:
            compositor = subprocess.Popen([
                "mutter", "--headless", "--virtual-monitor", "1920x1080",
                "--wayland", "--no-x11", "--wayland-display", "sharpr-test",
            ], env=env, stdout=log, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic() + 15
                while not (runtime / "sharpr-test").is_socket():
                    if compositor.poll() is not None or time.monotonic() >= deadline:
                        raise RuntimeError((runtime / "mutter.log").read_text())
                    time.sleep(.05)
                return subprocess.run(command, env=env).returncode
            finally:
                compositor.terminate()
                try:
                    compositor.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    compositor.kill()
                    compositor.wait()
    command = sys.argv[1:]
    if command[:1] == ["--"]:
        command = command[1:]
    if not command:
        raise SystemExit("Usage: gtk-offscreen.py -- COMMAND [ARGUMENT ...]")
    for program in ("mutter", "dbus-run-session"):
        if not shutil.which(program):
            raise SystemExit(f"{program} is required for off-screen GTK checks")
    with tempfile.TemporaryDirectory(prefix="sharpr-display-") as runtime:
        return subprocess.run([
            "dbus-run-session", "--", sys.executable, str(Path(__file__).resolve()),
            "--inner", runtime, *command,
        ]).returncode


if __name__ == "__main__":
    raise SystemExit(main())
