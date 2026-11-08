#!/usr/bin/env python3
"""A new game in the original under the private x32dbg, with no input: see docs/original-debug.md.

capture_newgame.py IDLE_FRAMES SECONDS
Needs x32dbg paused at the process start, with logfile.dp32 loaded. Runs with capture_c.py's script census probes to the menu, and at
the IDLE_FRAMES-th 0040F26A does what the confirmation's Yes does: InterfaceManager +0xB0 = 1, which 00578D70 reads, then a call of
005B5DF0 with 0040F26A as its return. It logs for SECONDS more, and leaves the game running: the caller stops it.
"""
import sys
import time

import x32
from capture_c import SCRIPTS

FRAME = 0x40F26A
NEW_GAME = 0x5B5DF0
MANAGER = 0xB3A6E0


def main():
    frames, seconds = int(sys.argv[1]), float(sys.argv[2])
    for _ in range(4):
        if x32.get("Register/Get", register="eip").lower() == "0x98787c":
            break
        x32.cmd("run")
        time.sleep(1)
    x32.cmd("bc")
    for address, text in SCRIPTS.items():
        if address == FRAME:
            continue
        target = hex(address)
        for command in (f"bp {target}", f"SetBreakpointCondition {target}, 0", f"SetBreakpointSilent {target}, 1",
                        f'SetBreakpointLog {target}, "{text}"'):
            assert '"success":true' in x32.cmd(command), command
    target = hex(FRAME)
    for command in (f"bp {target}", f"SetBreakpointCondition {target}, $breakpointcounter=={frames}",
                    f"SetBreakpointSilent {target}, 1", f'SetBreakpointLog {target}, "CP idle_frame"'):
        assert '"success":true' in x32.cmd(command), command
    x32.cmd("run")
    start = time.time()
    while x32.get("Register/Get", register="eip").lower() != hex(FRAME):
        if time.time() - start > 300:
            sys.exit("no pause at the frame")
        time.sleep(2)
    print("at the frame after", round(time.time() - start), "s")
    x32.cmd(f"bc {target}")
    for command in (f"[[{hex(MANAGER)}]+0xB0]=1", "esp=esp-4", f"[esp]={target}", f"eip={hex(NEW_GAME)}"):
        assert '"success":true' in x32.cmd(command), command
    x32.cmd(f'log "CP new_game"')
    # Each iteration from here: the status, the player's cell, and the TES's interior.
    for command in (f"bp {target}", f"SetBreakpointCondition {target}, 0", f"SetBreakpointSilent {target}, 1",
                    f'SetBreakpointLog {target}, "FRAME status={{x:[[{hex(MANAGER)}]+8]&0xFF}} cell={{x:[[0xB333C4]+0x40]}} pos={{x:[[0xB333C4]+0x2C]}}"'):
        assert '"success":true' in x32.cmd(command), command
    x32.cmd("run")
    time.sleep(seconds)
    print("eip", x32.get("Register/Get", register="eip"), "running", x32.running())


if __name__ == "__main__":
    main()
