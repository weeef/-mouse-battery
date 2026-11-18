"""
Drawing: the taskbar widget ("<mouse outline> 93%", Windows 11 tray style) and the small
pixel-art tray icon.
"""
from __future__ import annotations

import ctypes
import os
from ctypes import wintypes

from PIL import Image, ImageDraw, ImageFont

from .common import LOW_BATTERY, load_config, log, save_config

user32 = ctypes.windll.user32


# ---------------------------------------------------------------- Windows helpers

def window_rect(hwnd):
    r = wintypes.RECT()
    if hwnd and user32.GetWindowRect(hwnd, ctypes.byref(r)):
        return r.left, r.top, r.right, r.bottom
    return None


def screen_size():
    return user32.GetSystemMetrics(0), user32.GetSystemMetrics(1)


def taskbar_rect():
    r = window_rect(user32.FindWindowW("Shell_TrayWnd", None))
    if r and r[2] > r[0] and r[3] > r[1]:
        return r
    w, h = screen_size()
    return 0, h - 48, w, h


def tray_left():
    """Left edge of the notification area (clock + tray icons), if Windows tells us."""
    tb = user32.FindWindowW("Shell_TrayWnd", None)
    r = window_rect(user32.FindWindowExW(tb, 0, "TrayNotifyWnd", None)) if tb else None
    return r[0] if r and r[2] > r[0] else None


def dpi_scale():
    try:
        return user32.GetDpiForSystem() / 96.0
    except Exception:
        return 1.0


def tray_icon_size():
    try:
        return max(16, user32.GetSystemMetrics(49))      # SM_CXSMICON
    except Exception:
        return 16


def fullscreen_app_active():
    fg = user32.GetForegroundWindow()
    if not fg:
        return False
    cls = ctypes.create_unicode_buffer(64)
    user32.GetClassNameW(fg, cls, 64)
    if cls.value in ("Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd", "TkTopLevel"):
        return False
    r = window_rect(fg)
    w, h = screen_size()
    return bool(r) and r[0] <= 0 and r[1] <= 0 and r[2] >= w and r[3] >= h


# ---------------------------------------------------------------- taskbar widget image

_FONTS = {}


def widget_font(px):
    if px not in _FONTS:
        fonts = os.path.join(os.environ.get("WINDIR", r"C:\Windows"), "Fonts")
        for name in ("seguisb.ttf", "SegUIVar.ttf", "segoeui.ttf", "arial.ttf"):
            try:
                _FONTS[px] = ImageFont.truetype(os.path.join(fonts, name), px)
                break
            except OSError:
                continue
        else:
            _FONTS[px] = ImageFont.load_default()
    return _FONTS[px]


def widget_color(level, charging, asleep, bg):
    light_bg = (0.299 * bg[0] + 0.587 * bg[1] + 0.114 * bg[2]) > 140
    if level is None or asleep:
        return (110, 110, 110) if light_bg else (160, 160, 160)
    if charging:
        return (0, 103, 192) if light_bg else (96, 180, 255)
    if level <= LOW_BATTERY:
        return (196, 43, 28) if light_bg else (255, 99, 99)
    return (20, 20, 20) if light_bg else (255, 255, 255)


def render_widget(entries, bg, scale, height):
    """
    entries: list of (level, charging, asleep), one per mouse.
    Draws each as a mouse outline + "93%" on the taskbar colour, supersampled 4x.
    """
    entries = entries or [(None, False, False)]
    ss = 4
    s = scale * ss
    font = widget_font(round(14 * s))
    pad, icon_w, icon_h, gap, between = 6 * s, 11 * s, 16 * s, 5 * s, 12 * s

    items = []
    for level, charging, asleep in entries:
        text = "--%" if level is None else f"{level}%"
        items.append((text, font.getbbox(text), widget_color(level, charging, asleep, bg)))

    W = round(2 * pad + sum(icon_w + gap + (bb[2] - bb[0]) for _, bb, _ in items)
              + between * (len(items) - 1))
    H = round(height * ss)
    img = Image.new("RGB", (W, H), bg)
    d = ImageDraw.Draw(img)
    lw = max(1, round(1.5 * s))                    # ~1.5px strokes, like the tray icons

    x = pad
    for text, (l, t, r, b), color in items:
        y0 = (H - icon_h) / 2
        d.rounded_rectangle((x, y0, x + icon_w, y0 + icon_h), radius=icon_w / 2, outline=color, width=lw)
        cx = x + icon_w / 2
        d.line((cx, y0, cx, y0 + icon_h * 0.36), fill=color, width=lw)
        x += icon_w + gap
        d.text((x - l, (H - (b - t)) / 2 - t), text, font=font, fill=color)
        x += (r - l) + between
    return img.resize((max(1, round(W / ss)), max(1, round(H / ss))), Image.LANCZOS)


# ---------------------------------------------------------------- tray icon image
# The tray shows a 16x16 icon at 100% scaling, so it is drawn pixel by pixel on a 16x16 grid.

MOUSE = ["...#####...",                    # 11 x 16 mouse, '#' = body
         ".#########.",
         ".#########."] + ["###########"] * 10 + [".#########.", ".#########.", "...#####..."]
MOUSE_X = 2
SPLIT = 5                                  # row of the line between buttons and palm
DIGITS = {                                 # 3 x 5 pixel digits ("1" is 1 pixel wide)
    "0": "111101101101111", "1": "11111", "2": "111001111100111", "3": "111001111001111",
    "4": "101101111001001", "5": "111100111001111", "6": "111100111101111",
    "7": "111001001001001", "8": "111101111101111", "9": "111101111001111", "-": "000000111000000",
}


def tray_color(level, charging, asleep):
    if level is None or asleep:
        return (150, 150, 150)
    if charging:
        return (80, 170, 255)
    if level <= LOW_BATTERY:
        return (240, 70, 70)
    if level <= 35:
        return (245, 190, 50)
    return (90, 210, 110)


def make_tray_icon(level, charging=False, asleep=False):
    img = Image.new("RGBA", (16, 16), (0, 0, 0, 0))
    color = tray_color(level, charging, asleep) + (255,)

    def body(x, y):
        return 0 <= y < len(MOUSE) and 0 <= x < len(MOUSE[0]) and MOUSE[y][x] == "#"

    for y, row in enumerate(MOUSE):
        for x in range(len(row)):
            if body(x, y) and not all(body(x + dx, y + dy) for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1))):
                img.putpixel((MOUSE_X + x, y), color)
    for x in range(1, len(MOUSE[0]) - 1):
        img.putpixel((MOUSE_X + x, SPLIT), color)
    for y in range(SPLIT):
        img.putpixel((MOUSE_X + len(MOUSE[0]) // 2, y), color)

    text = "--" if level is None else str(level)
    widths = [len(DIGITS[c]) // 5 for c in text]
    x = MOUSE_X + (len(MOUSE[0]) - (sum(widths) + len(text) - 1) + 1) // 2
    for ch, w in zip(text, widths):
        for i, on in enumerate(DIGITS[ch]):
            if on == "1":
                img.putpixel((x + i % w, 8 + i // w), (255, 255, 255, 255))
        x += w + 1

    size = tray_icon_size()                # bigger tray slot at >100% scaling; keep hard pixels
    return img.resize((size, size), Image.NEAREST) if size != 16 else img


# ---------------------------------------------------------------- taskbar widget window

class TaskbarWidget:
    """A borderless always-on-top window on the taskbar. Runs tkinter on the main thread
    and reads the app's state once a second."""

    def __init__(self, app):
        import tkinter as tk
        self.tk = tk
        self.app = app
        self.cfg = load_config()
        self.scale = dpi_scale()
        self.root = tk.Tk()
        self.root.withdraw()
        self.root.overrideredirect(True)
        self.root.attributes("-topmost", True)
        self.label = tk.Label(self.root, bd=0, highlightthickness=0, cursor="hand2")
        self.label.pack()
        self.photo = None
        self.drawn = None
        self.bg = (32, 32, 32)
        self.ticks = 0
        self.size = (0, 0)
        self.drag_dx = None
        self.label.bind("<ButtonPress-1>", self.on_press)
        self.label.bind("<B1-Motion>", self.on_drag)
        self.label.bind("<ButtonRelease-1>", self.on_release)
        self.label.bind("<Double-Button-1>", lambda e: self.app.wake.set())
        self.label.bind("<Button-3>", self.on_menu)

    # -- placement
    def hwnd(self):
        try:
            return int(self.root.wm_frame(), 16)
        except Exception:
            return self.root.winfo_id()

    def target_xy(self, w, h):
        tl, tt, tr, tb = taskbar_rect()
        if "right_offset" in self.cfg:
            x = tr - int(self.cfg["right_offset"])
        else:
            left = tray_left()
            x = (left if left else tr - round(260 * self.scale)) - w - round(4 * self.scale)
        x = min(max(x, tl), tr - w)
        return x, tt + (tb - tt - h) // 2

    def sample_bg(self):
        """Read the taskbar colour just beside the widget so the text blends in."""
        try:
            from PIL import ImageGrab
            tl, tt, tr, tb = taskbar_rect()
            if tb - tt > 200:                  # vertical taskbar: not supported, keep default
                return
            w = self.size[0] or round(60 * self.scale)
            x, _ = self.target_xy(w, 1)
            px = x - 3 if x - 3 > tl else min(x + w + 3, tr - 1)
            py = tt + (tb - tt) // 2
            c = ImageGrab.grab(bbox=(px, py, px + 1, py + 1), all_screens=True).getpixel((0, 0))
            self.bg = tuple(c[:3])
        except Exception as e:
            log("bg sample failed:", e)

    # -- mouse interaction
    def on_press(self, e):
        self.drag_dx = e.x_root - self.root.winfo_x()

    def on_drag(self, e):
        if self.drag_dx is None:
            return
        tl, tt, tr, tb = taskbar_rect()
        w, h = self.size
        x = min(max(e.x_root - self.drag_dx, tl), tr - w)
        self.root.geometry(f"+{x}+{tt + (tb - tt - h) // 2}")

    def on_release(self, _e):
        if self.drag_dx is not None:
            self.cfg = load_config()
            self.cfg["right_offset"] = taskbar_rect()[2] - self.root.winfo_x()
            save_config(self.cfg)
            self.drawn = None                  # resample the background at the new spot
            self.ticks = 0
        self.drag_dx = None

    def on_menu(self, e):
        m = self.tk.Menu(self.root, tearoff=0)
        for line in self.app.status_lines():
            m.add_command(label=line, state="disabled")
        m.add_separator()
        m.add_command(label="Refresh now", command=self.app.wake.set)
        m.add_command(label="Reset position", command=self.reset_position)
        m.add_command(label="Hide widget (show again from tray)",
                      command=lambda: self.app.set_widget_visible(False))
        m.add_separator()
        m.add_command(label="Quit", command=self.app.quit)
        try:
            m.tk_popup(e.x_root, e.y_root)
        finally:
            m.grab_release()

    def reset_position(self):
        self.cfg = load_config()
        self.cfg.pop("right_offset", None)
        save_config(self.cfg)
        self.drawn = None
        self.ticks = 0

    # -- refresh loop
    def tick(self):
        if not self.app.running:
            self.root.destroy()
            return
        try:
            self.update()
        except Exception as e:
            log("widget error:", e)
        self.root.after(1000, self.tick)

    def update(self):
        if not self.app.widget_visible or fullscreen_app_active():
            self.root.withdraw()
            return

        if self.ticks % 10 == 0:               # taskbar colour changes rarely (theme switch)
            self.sample_bg()
        self.ticks += 1

        entries = self.app.widget_entries()
        tl, tt, tr, tb = taskbar_rect()
        height = min(tb - tt, round(40 * self.scale)) / self.scale
        key = (tuple(entries), self.bg, height)
        if key != self.drawn:
            from PIL import ImageTk
            img = render_widget(entries, self.bg, self.scale, height)
            self.photo = ImageTk.PhotoImage(img)
            self.label.configure(image=self.photo)
            self.size = img.size
            self.drawn = key

        if self.drag_dx is None:
            x, y = self.target_xy(*self.size)
            self.root.geometry(f"{self.size[0]}x{self.size[1]}+{x}+{y}")
        self.root.deiconify()
        # the taskbar pushes itself above other topmost windows when clicked; climb back on top
        user32.SetWindowPos(self.hwnd(), -1, 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010)

    def run(self):
        self.root.after(200, self.tick)
        self.root.mainloop()
