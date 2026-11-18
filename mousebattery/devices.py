"""
Battery readers for the supported mice. Each reader returns a list of Reading objects:
one per mouse that is plugged in, with level=None when the mouse is present but silent
(usually asleep).

WLmouse (Beast X family), VID 0x36A7 - community reverse-engineered, see HaloBattery (MIT):
  * request: feature report id 0, payload 00 00 02 02 00 83 (padded to 64 bytes)
  * reply:   a1|a2 00 02 02 00 83 <charging> <battery%>
  * fallback: the mouse sometimes sends an input report 03 00 <battery%> <charging>

Attack Shark X3, VID 0x1D57 - community reverse-engineered (x3ctl, attack-shark-mac-battery):
  * the 2.4G dongle (PID 0xFA60) pushes an input report every few seconds while the mouse
    is awake, no request needed:  03 xx 40 01 <level>   (level 1-10 = 10%-100%)
  * when the USB-C cable is plugged in the mouse also shows up as PID 0xFA61 -> charging
"""
from __future__ import annotations

import re
import time
from dataclasses import dataclass

import hid

from .common import log


@dataclass
class Reading:
    key: str                  # stable id for the mouse ("wlmouse", "attackshark")
    name: str                 # display name
    level: int | None         # battery %, None when the mouse did not answer
    charging: bool = False


def hexdump(data, n=16):
    return " ".join(f"{b:02x}" for b in list(data)[:n])


def log_infos(infos):
    for d in infos:
        log(f"  found {d['vendor_id']:04x}:{d['product_id']:04x} iface={d.get('interface_number')} "
            f"usage={d.get('usage_page', 0):04x}:{d.get('usage', 0):04x} "
            f"'{(d.get('product_string') or '').strip()}'")


def listen(paths, seconds, parse):
    """Open several HID collections at once and wait for a report that `parse` accepts."""
    devs = []
    for path in dict.fromkeys(paths):                     # de-duplicate, keep order
        dev = hid.device()
        try:
            dev.open_path(path)
            dev.set_nonblocking(True)
            devs.append(dev)
        except (OSError, IOError) as e:
            log("   open failed:", e)
    try:
        end = time.time() + seconds
        while devs and time.time() < end:
            for dev in devs:
                try:
                    data = dev.read(64)
                except (OSError, ValueError):
                    data = None
                if data:
                    log("   report:", hexdump(data))
                    result = parse(data)
                    if result is not None:
                        return result
            time.sleep(0.02)
    finally:
        for dev in devs:
            dev.close()
    return None


# ---------------------------------------------------------------- WLmouse (Beast X family)

WLMOUSE_VID = 0x36A7
WLMOUSE_NAMES = {0xA868: "Beast X Mini Pro", 0xA887: "Beast X", 0xA880: "Beast X Max"}
WLMOUSE_QUERY = [0x00, 0x00, 0x02, 0x02, 0x00, 0x83]


def wl_parse_feature(resp):
    r = list(resp or [])
    for i in range(5, len(r) - 2):
        if r[i] == 0x83 and r[i - 1] == 0x00 and r[i - 2] == 0x02 and r[i - 5] in (0xA1, 0xA2):
            charging, level = r[i + 1], r[i + 2]
            if 0 <= level <= 100:
                return level, bool(charging)
    return None


def wl_parse_heartbeat(data):
    d = list(data or [])
    for off in (0, 1):
        if len(d) >= off + 4 and d[off] == 0x03 and d[off + 1] == 0x00 and 0 <= d[off + 2] <= 100:
            return d[off + 2], bool(d[off + 3])
    return None


def wl_query(path):
    dev = hid.device()
    try:
        dev.open_path(path)
    except (OSError, IOError) as e:
        log("   open failed:", e)
        return None
    try:
        try:
            dev.send_feature_report([0x00] + WLMOUSE_QUERY + [0x00] * (64 - len(WLMOUSE_QUERY)))
        except (OSError, ValueError) as e:
            log("   send failed:", e)
        for _ in range(15):
            time.sleep(0.05)
            for length in (65, 64):
                try:
                    resp = dev.get_feature_report(0, length)
                except (OSError, ValueError):
                    resp = None
                result = wl_parse_feature(resp) if resp else None
                if result:
                    log("   reply:", hexdump(resp, 12))
                    return result
        return None
    finally:
        dev.close()


def wl_name(pid, infos):
    name = WLMOUSE_NAMES.get(pid) or (infos[0].get("product_string") or "").strip()
    for junk in ("WLMOUSE", " 8K", " 4K", " 1K", " Receiver", " Dongle", " Wireless"):
        name = re.sub(re.escape(junk), "", name, flags=re.I)
    return name.strip() or "Beast X"


def read_wlmouse() -> list[Reading]:
    infos = hid.enumerate(WLMOUSE_VID)
    if not infos:
        return []
    log_infos(infos)

    groups = {}
    for d in infos:
        groups.setdefault(d["product_id"], []).append(d)

    # a mouse on its cable appears with its own PID next to the receiver;
    # prefer the reading that says charging
    best, fallback_name = None, None
    for pid, ifaces in groups.items():
        name = wl_name(pid, ifaces)
        fallback_name = fallback_name or name
        vendor = [d for d in ifaces if d.get("usage_page") == 0xFFFF]
        vendor.sort(key=lambda d: 0 if d.get("usage") == 0 else 1)
        if not vendor:
            vendor = [d for d in ifaces if d.get("usage_page", 0) >= 0xFF00]
        result = None
        for d in vendor:
            log(f"  asking {pid:04x} usage={d.get('usage_page', 0):04x}:{d.get('usage', 0):04x}")
            result = wl_query(d["path"])
            if result:
                break
        if not result and vendor:
            result = listen([d["path"] for d in vendor[:2]], 1.0, wl_parse_heartbeat)
        if result and (best is None or (result[1] and not best[1])):
            best = (result[0], result[1], name)

    if best:
        return [Reading("wlmouse", best[2], best[0], best[1])]
    return [Reading("wlmouse", fallback_name, None)]


# ---------------------------------------------------------------- Attack Shark X3

SHARK_VID = 0x1D57
SHARK_DONGLE_PIDS = {0xFA60}
SHARK_WIRED_PIDS = {0xFA61, 0xFA50}
SHARK_LISTEN_SECONDS = 6          # the dongle reports every ~2-4 s while the mouse is awake


def shark_parse(data):
    d = list(data or [])
    if len(d) >= 5 and d[0] == 0x03 and d[2] == 0x40 and d[3] == 0x01:
        raw = d[4]
        if 1 <= raw <= 10:                       # 1-10 scale, 10 = full
            return raw * 10
        if 0 <= raw <= 100:
            return raw
    return None


def shark_priority(d):
    page, usage = d.get("usage_page", 0), d.get("usage", 0)
    if page == 0x000A:
        return 0
    if (page, usage) == (0x0001, 0x0080):
        return 1
    if page >= 0xFF00:
        return 2
    return 3


def read_attack_shark() -> list[Reading]:
    infos = [d for d in hid.enumerate(SHARK_VID)
             if d["product_id"] in SHARK_DONGLE_PIDS | SHARK_WIRED_PIDS]
    if not infos:
        return []
    log_infos(infos)

    wired = any(d["product_id"] in SHARK_WIRED_PIDS for d in infos)
    # Windows keeps the plain mouse/keyboard collections to itself, so skip those
    usable = [d for d in infos
              if not (d.get("usage_page") == 0x0001 and d.get("usage") in (0x0002, 0x0006))]
    usable.sort(key=shark_priority)
    log(f"  listening for {SHARK_LISTEN_SECONDS}s on {len(usable)} collection(s)...")
    level = listen([d["path"] for d in usable], SHARK_LISTEN_SECONDS, shark_parse)
    return [Reading("attackshark", "Attack Shark X3", level, wired)]


# ---------------------------------------------------------------- all mice

READERS = (read_wlmouse, read_attack_shark)


def read_all() -> list[Reading]:
    readings = []
    for reader in READERS:
        try:
            readings.extend(reader())
        except Exception as e:                   # one broken driver must not hide the others
            log(f"{reader.__name__} failed:", e)
    return readings
