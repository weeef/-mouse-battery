"""
Mouse Battery - shows your wireless mouse's battery level on the Windows taskbar.

Run normally:            pythonw mouse_battery.pyw
Run a one-shot check:    python mouse_battery.pyw --debug
"""
from mousebattery.app import main

if __name__ == "__main__":
    main()
