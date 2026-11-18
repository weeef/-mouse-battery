"""Tray icon, polling loop, notifications and the "Start with Windows" setting."""
from __future__ import annotations

import ctypes
import os
import sys
import threading
import time

import pystray

from . import devices
from .common import APP_NAME, DEBUG, LOW_BATTERY, POLL_SECONDS, STALE_SECONDS, load_config, log, save_config
from .widget import TaskbarWidget, make_tray_icon


# ---------------------------------------------------------------- start with Windows

RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"
LEGACY_RUN_NAMES = ("Beast Battery",)            # v1 name; replaced so only one copy starts


def launch_command():
    if getattr(sys, "frozen", False):            # packaged .exe
        return f'"{sys.executable}"'
    pyw = sys.executable.replace("python.exe", "pythonw.exe")
    return f'"{pyw}" "{os.path.abspath(sys.argv[0])}"'


def startup_enabled(_item=None):
    try:
        import winreg
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY) as k:
            winreg.QueryValueEx(k, APP_NAME)
            return True
    except (ImportError, OSError):
        return False


def toggle_startup(icon, _item):
    import winreg
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY, 0, winreg.KEY_SET_VALUE) as k:
        if startup_enabled():
            winreg.DeleteValue(k, APP_NAME)
        else:
            winreg.SetValueEx(k, APP_NAME, 0, winreg.REG_SZ, launch_command())
    icon.update_menu()


def migrate_startup():
    """Replace an old "Beast Battery" startup entry with this app."""
    try:
        import winreg
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY, 0,
                            winreg.KEY_QUERY_VALUE | winreg.KEY_SET_VALUE) as k:
            found = False
            for name in LEGACY_RUN_NAMES:
                try:
                    winreg.QueryValueEx(k, name)
                    winreg.DeleteValue(k, name)
                    found = True
                except OSError:
                    pass
            if found:
                winreg.SetValueEx(k, APP_NAME, 0, winreg.REG_SZ, launch_command())
    except (ImportError, OSError) as e:
        log("startup migration failed:", e)


# ---------------------------------------------------------------- per-mouse state

class Mouse:
    """Battery state of one mouse, with low-battery and fully-charged alerts."""

    def __init__(self, name):
        self.name = name
        self.last = None              # (level, charging, timestamp)
        self.warned_low = False
        self.warned_full = False

    def update(self, reading, notify):
        self.name = reading.name or self.name
        level, charging = reading.level, reading.charging
        if level is None:
            if self.last and charging:                      # silent while on the cable
                self.last = (self.last[0], True, time.time())
            elif self.last and time.time() - self.last[2] >= STALE_SECONDS:
                self.last = None
            return
        self.last = (level, charging, time.time())
        if level <= LOW_BATTERY and not charging and not self.warned_low:
            notify(f"{self.name} battery is at {level}%. Time to charge.")
            self.warned_low = True
        if charging or level > LOW_BATTERY + 5:
            self.warned_low = False
        if charging and level >= 100 and not self.warned_full:
            notify(f"{self.name} is fully charged.")
            self.warned_full = True
        if not charging:
            self.warned_full = False

    def asleep(self):
        return bool(self.last) and time.time() - self.last[2] > POLL_SECONDS * 1.5

    def row(self):
        return self.last[0], self.last[1], self.asleep()

    def text(self):
        if not self.last:
            return f"{self.name}: waiting for the mouse (move it to wake it)"
        level, charging, _ = self.last
        state = " - charging" if charging else (" (asleep, last reading)" if self.asleep() else "")
        return f"{self.name}: {level}%{state}"


# ---------------------------------------------------------------- app

class App:
    def __init__(self):
        self.wake = threading.Event()
        self.running = True
        self.ready = False
        self.lock = threading.Lock()
        self.mice: dict[str, Mouse] = {}
        self.widget_visible = load_config().get("visible", True)
        self.icon = pystray.Icon(
            APP_NAME,
            make_tray_icon(None),
            f"{APP_NAME}: looking...",
            menu=pystray.Menu(lambda: self.menu_items()),
        )

    # -- state shared with the widget (read from the tkinter thread)
    def snapshot(self):
        with self.lock:
            return list(self.mice.values())

    def status_lines(self):
        mice = self.snapshot()
        return [m.text() for m in mice] or ["No supported mouse found"]

    def widget_entries(self):
        return [m.row() for m in self.snapshot() if m.last]

    # -- tray
    def menu_items(self):
        for line in self.status_lines():
            yield pystray.MenuItem(line, None, enabled=False)
        yield pystray.Menu.SEPARATOR
        yield pystray.MenuItem("Refresh now", lambda: self.wake.set(), default=True)
        yield pystray.MenuItem("Show on taskbar", lambda: self.set_widget_visible(not self.widget_visible),
                               checked=lambda _: self.widget_visible)
        yield pystray.MenuItem("Start with Windows", toggle_startup, checked=startup_enabled)
        yield pystray.Menu.SEPARATOR
        yield pystray.MenuItem("Quit", lambda: self.quit())

    def set_widget_visible(self, visible):
        self.widget_visible = visible
        cfg = load_config()
        cfg["visible"] = visible
        save_config(cfg)
        try:
            self.icon.update_menu()
        except Exception:
            pass

    def notify(self, message):
        try:
            if self.ready:
                self.icon.notify(message, APP_NAME)
        except Exception as e:
            log("notify failed:", e)

    def redraw(self):
        shown = [m for m in self.snapshot() if m.last]
        if shown:
            lowest = min(shown, key=lambda m: m.last[0])        # tray icon shows the emptiest mouse
            self.icon.icon = make_tray_icon(*lowest.row())
            self.icon.title = "\n".join(m.text() for m in shown)[:127]   # Windows tooltip limit
        else:
            self.icon.icon = make_tray_icon(None)
            self.icon.title = f"{APP_NAME}: no mouse found"
        self.icon.update_menu()

    # -- polling
    def poll_once(self):
        readings = devices.read_all()
        with self.lock:
            present = {r.key for r in readings}
            for key in [k for k in self.mice if k not in present]:      # receiver unplugged
                del self.mice[key]
            for r in readings:
                self.mice.setdefault(r.key, Mouse(r.name)).update(r, self.notify)

    def poll_loop(self):
        while self.running:
            try:
                self.poll_once()
            except Exception as e:                      # never let the tray die
                log("poll error:", e)
            try:
                self.redraw()
            except Exception as e:
                log("redraw error:", e)
            self.wake.wait(POLL_SECONDS)
            self.wake.clear()

    def quit(self):
        self.running = False
        self.wake.set()
        self.icon.stop()

    def run(self):
        def setup(icon):
            icon.visible = True
            self.ready = True
            threading.Thread(target=self.poll_loop, daemon=True).start()

        # tray icon on a background thread, taskbar widget (tkinter) on the main thread
        threading.Thread(target=lambda: self.icon.run(setup), daemon=True).start()
        TaskbarWidget(self).run()


def main():
    if DEBUG:
        print("Looking for supported mice (move the mouse so it is awake)...")
        readings = devices.read_all()
        for r in readings:
            print(f"{r.name}: " + ("no answer" if r.level is None else
                                   f"{r.level}%{' (charging)' if r.charging else ''}"))
        if not readings:
            print("No supported mouse receiver found.")
        return
    try:                                  # sharp text and the real tray icon size
        ctypes.windll.user32.SetProcessDPIAware()
    except Exception:
        pass
    migrate_startup()
    App().run()
