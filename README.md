# Mouse Battery

Shows your wireless gaming mouse's battery level right on the Windows taskbar, in the
same thin outline style as the Windows 11 tray icons:

```
🖱 93%
```

- White normally, **red** at 15% or below (with a notification), **blue** while charging
  (notification at 100%), **grey** while the mouse is asleep (last known value).
- Several supported mice plugged in at once are shown side by side.
- Hides itself while a fullscreen game or video is in front.
- A small tray icon is kept for the menu and notifications.

## Supported mice

| Mouse | Receiver (VID:PID) | Status |
|---|---|---|
| WLmouse Beast X Mini Pro | `36A7:A868` | Tested |
| WLmouse Beast X / Beast X Max | `36A7:A887` / `36A7:A880` | Same protocol, untested |
| Attack Shark X3 | `1D57:FA60` (dongle), `1D57:FA61` (cable) | Community protocol, untested |

The Attack Shark only reports its battery while it is awake (about every 2-4 seconds), so
move the mouse if it shows `--%`. Its level comes in 10% steps. While it charges on the
cable the dongle goes quiet, so the last level is shown in blue.

## Setup (Windows 10/11)

1. Install Python 3.10+ from [python.org](https://www.python.org/) and tick
   **Add python.exe to PATH**.
2. Double-click `run.bat`.
3. The widget appears on the taskbar next to the tray icons. **Drag** it to move it,
   **double-click** to refresh, **right-click** for the menu.
4. Right-click the tray icon and tick **Start with Windows**.

### Standalone .exe

`build_exe.bat` builds `dist\MouseBattery.exe`, which runs without Python. Start it
once and tick **Start with Windows** in its tray menu.

## Not working?

- Close the mouse's own software / web driver first; it can keep the receiver busy.
- Run `debug.bat` and move the mouse while it runs. It lists the receivers it finds and
  the raw reports it receives, so please include that output when opening an issue.

## Adding a mouse

Each mouse family is a small reader function in [`mousebattery/devices.py`](mousebattery/devices.py)
that returns `Reading(key, name, level, charging)`. Add one and list it in `READERS`.

## Project layout

```
mouse_battery.pyw        entry point
mousebattery/
  app.py                 tray icon, polling, notifications, start with Windows
  devices.py             HID battery readers for each mouse
  widget.py              taskbar widget and tray icon drawing
  common.py              settings, logging, config file
```

Settings (widget position, visibility) live in `%APPDATA%\MouseBattery\settings.json`.

## Credits

Protocol details come from community reverse engineering:
- WLmouse: HaloBattery (MIT)
- Attack Shark X3: [x3ctl](https://github.com/Elektrik1/x3ctl) and
  [attack-shark-mac-battery](https://github.com/renato-minuar/attack-shark-mac-battery)
