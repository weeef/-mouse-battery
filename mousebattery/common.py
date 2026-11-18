"""Settings, logging and the small JSON config file shared by the rest of the app."""
from __future__ import annotations

import json
import os
import sys

APP_NAME = "Mouse Battery"
POLL_SECONDS = 60          # how often the mice are asked for their battery level
LOW_BATTERY = 15           # % at which the text turns red and a notification is shown
STALE_SECONDS = 30 * 60    # keep showing a sleeping mouse's last reading for this long

DEBUG = "--debug" in sys.argv

_APPDATA = os.environ.get("APPDATA") or os.path.expanduser("~")
CONFIG_DIR = os.path.join(_APPDATA, "MouseBattery")
CONFIG_FILE = os.path.join(CONFIG_DIR, "settings.json")
LEGACY_CONFIG_FILE = os.path.join(_APPDATA, "BeastBattery", "widget.json")   # v1 (Beast Battery)


def log(*args):
    if DEBUG:
        print(*args, flush=True)


def load_config() -> dict:
    for path in (CONFIG_FILE, LEGACY_CONFIG_FILE):
        try:
            with open(path, "r", encoding="utf-8") as f:
                return json.load(f)
        except (OSError, ValueError):
            continue
    return {}


def save_config(cfg: dict) -> None:
    try:
        os.makedirs(CONFIG_DIR, exist_ok=True)
        with open(CONFIG_FILE, "w", encoding="utf-8") as f:
            json.dump(cfg, f, indent=2)
    except OSError as e:
        log("config save failed:", e)
