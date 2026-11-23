# Mouse Battery

Shows your wireless gaming mouse's battery level right on the Windows taskbar, in the
same thin outline style as the Windows 11 tray icons:

```
🖱 93%
```

- White normally, **red** at 15% or below (with a notification), **blue** while charging
  (notification at 100%), **grey** while the mouse is asleep (last known value).
- The Beast X family reports 100% as soon as the cable goes in, so while charging the level
  is estimated from the last unplugged reading (about 90 minutes for a full charge).
- Several supported mice plugged in at once are shown side by side.
- Hides itself while a fullscreen game or video is in front.
- A small tray icon is kept for the menu and notifications.
- A single ~0.5 MB native exe written in Rust; no runtime to install. Only one copy runs at a time.

## Supported mice

| Mouse | Receiver (VID:PID) | Status |
|---|---|---|
| WLmouse Beast X Mini Pro | `36A7:A868` | Tested |
| WLmouse Beast X / Beast X Max | `36A7:A887` / `36A7:A880` | Same protocol, untested |
| Attack Shark X3 | `1D57:FA60` (dongle), `1D57:FA61` (cable) | Community protocol, untested |

The Attack Shark only reports its battery while it is awake (about every 2-4 seconds), so
move the mouse if it shows `--%`. Its level comes in 10% steps. While it charges on the
cable the dongle goes quiet, so the last level is shown in blue.

## Use

1. Run `MouseBattery.exe`.
2. The widget appears on the taskbar next to the tray icons. **Drag** it to move it,
   **double-click** to refresh, **right-click** for the menu.
3. Right-click it (or the tray icon) and tick **Start with Windows**.

## Build

Install [Rust](https://rustup.rs) (MSVC toolchain, which needs the Visual Studio C++ build
tools), then run `build_exe.bat`, or:

```
cargo build --release
```

The exe is `target\release\MouseBattery.exe`; `build_exe.bat` also copies it to `dist\`.
Run the parser tests with `cargo test`.

## Not working?

- Close the mouse's own software / web driver first; it can keep the receiver busy.
- Run `debug.bat` (or `MouseBattery.exe --debug` from a terminal) and move the mouse while
  it runs. It lists the receivers it finds and the raw reports it receives, so please
  include that output when opening an issue.

## Adding a mouse

Each mouse family is a reader function in [`src/devices.rs`](src/devices.rs) that returns
`Reading { key, name, level, charging }`. Add one and call it from `read_all`.

## Project layout

```
src/
  main.rs        entry point, --debug mode, single-instance check
  ui.rs          widget window, tray icon, menu, polling thread
  devices.rs     HID battery readers for each mouse
  state.rs       per-mouse state, low/full notifications
  render.rs      widget and tray icon drawing
  win.rs         Win32 helpers
  config.rs      settings file
  startup.rs     "Start with Windows"
```

Settings (widget position, visibility) live in `%APPDATA%\MouseBattery\settings.json`.

## Credits

Protocol details come from community reverse engineering:
- WLmouse: HaloBattery (MIT)
- Attack Shark X3: [x3ctl](https://github.com/Elektrik1/x3ctl) and
  [attack-shark-mac-battery](https://github.com/renato-minuar/attack-shark-mac-battery)
