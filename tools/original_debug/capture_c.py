#!/usr/bin/env python3
"""Capture set C under the private x32dbg: logging breakpoints, about 1 ms a hit. See docs/original-debug.md.

Needs x32dbg paused at the process start with logfile.dp32 loaded. Lines land in the prefix's C:\\x32log.txt.
Polling over HTTP also logs, so it only reads the file while the game runs.
"""
import os
import sys
import time

import x32

LOG = os.path.expanduser("~/.wine-oblivion-capture/drive_c/x32log.txt")
POOLS = "p10={x:[[0xB33080+0x10]+0x44]} p14={x:[[0xB33080+0x14]+0x44]}"
PROBES = {
    # _srand's and _rand's retn: ecx is the seed, or the new _holdrand.
    0x9859DC: "SRAND tid={x:tid()} seed={x:ecx} ret={x:[esp]}",
    0x9859FE: "R tid={x:tid()} st={x:ecx} ret={x:[esp]}",
    # The editor ID map's only load insert. eax key, edi form, count before.
    0x44F2D9: "EDI key={x:eax} id={x:[edi+0xC]} type={x:[edi+4]&0xFF} n={x:[0xB06170]} "
              + POOLS + " text={s:eax}",
    0x44F2DE: "EDA n={x:[0xB06170]}",
    # 00680090, the door map's registration.
    0x6800D3: "DOOR_GATED ref={x:esi} id={x:[esi+0xC]}",
    0x6800E0: "DOOR_680A00 ref={x:esi} id={x:[esi+0xC]}",
    0x6800EE: "DOOR_BUILT node={x:esi}",
    # A cell's override frees its name; the EDID chunk builds a new extra.
    0x4C9780: "CELLFREE this={x:ecx} id={x:[ecx+0xC]} " + POOLS,
    0x4245E0: "EXTRANEW ret={x:[esp]} " + POOLS,
    # 0074FF37: the rand value and +0x10, by thread.
    0x74FF37: "X87 tid={x:tid()} rand={x:eax} self={x:edi} f10={x:[edi+0x10]}",
    # 0040FDA0's exit code read, and who asked. 103h means the intro thread still runs.
    0x40FDBB: "INTRO code={x:[esp]} ret={x:[esp+4]}",
    # 00579260's draw, past its gates.
    0x57928B: "LMDRAW",
    # Phase edges, for the stream.
    0x4067CC: "CP load_end n={x:[0xB06170]}",
    0x40EBB9: "CP world_setup_end",
    0x40EE7A: "CP phase16_start",
    0x40EF54: "CP phase16_end",
}
# Set "progress": the loading bar's feed, in its own short run.
PROGRESS = {
    0x57B950: "PROG57 ret={x:[esp]} section={x:[esp+4]} pct={x:[esp+8]}",
    0x5ADEC0: "SEC ret={x:[esp]} section={x:[esp+4]} pct={x:[esp+8]} menu={x:[0xB3B3D8]}",
    0x5ADAF6: "DISP_RENDER",
    0x5ADE80: "CREATE_RENDER",
    0x57928B: "LMDRAW",
    0x4067CC: "CP load_end",
    0x40EBB9: "CP world_setup_end",
    0x40EF54: "CP phase16_end",
}
# Set "models": what the IO thread loads, and from where. The path twice, since {s:} can fail on fresh heap.
MODELS = {
    0x43B420: "LOADMODEL tid={x:tid()} ret={x:[esp]} path={x:[esp+8]}",
    0x437250: "QUEUED tid={x:tid()} ret={x:[esp]} stack={mem;180@esp}",
    0x6F9980: "STREAM tid={x:tid()} ret={x:[esp]} a1={x:[esp+4]} {s:[esp+4]} {mem;50@[esp+4]}",
    0x74FF37: "X87 tid={x:tid()} rand={x:eax} self={x:edi}",
    0x4067CC: "CP load_end",
    0x406944: "CP drain_end",
    0x40EB9A: "CP ai_window",
    0x40EBB9: "CP world_setup_end",
    0x40EE7A: "CP phase16_start",
    0x40EF54: "CP phase16_end",
}
# Set "scripts": every script run and every handler the script VM calls, from boot through the first idle frame.
SCRIPTS = {
    0x9859DC: "SRAND tid={x:tid()} seed={x:ecx} ret={x:[esp]}",
    0x517330: "RUN script={x:[esp+4]} id={x:[[esp+4]+0xC]} ret={x:[esp]}",
    0x516AF3: "BLOCK target={x:ecx}",
    0x5172C6: "CMD target={x:ecx}",
    0x4FADD5: "FN target={x:eax}",
    0x4FACA9: "VAL target={x:eax}",
    0x4067CC: "CP load_end",
    0x40EBB9: "CP world_setup_end",
    0x40EF54: "CP phase16_end",
    0x40F0E0: "CP phase21_end",
    0x40F26A: "CP idle_frame_start",
}
# "long" lets the idle loop run on, for what its later frames reach; the caller stops the game.
STOP = None if "long" in sys.argv else 0x40F275 if "scripts" in sys.argv else 0x40F16A


def arm():
    # x32dbg restores a program's breakpoints from its database.
    x32.cmd("bc")
    for address, text in (SCRIPTS if "scripts" in sys.argv else MODELS if "models" in sys.argv else PROGRESS if "progress" in sys.argv else PROBES).items():
        target = hex(address)
        for command in (f"bp {target}", f"SetBreakpointCondition {target}, 0", f"SetBreakpointSilent {target}, 1",
                        f'SetBreakpointLog {target}, "{text}"'):
            assert '"success":true' in x32.cmd(command), command
    if STOP is not None:
        assert '"success":true' in x32.cmd(f"bp {hex(STOP)}")


def main():
    limit = float(sys.argv[1]) if len(sys.argv) > 1 else 300
    # System breakpoint, then entry.
    for _ in range(4):
        eip = x32.get("Register/Get", register="eip")
        if eip.lower() == "0x98787c":
            break
        x32.cmd("run")
        time.sleep(1)
    print("entry", x32.get("Register/Get", register="eip"))
    arm()
    start = time.time()
    x32.cmd("run")
    last = -1
    while time.time() - start < limit:
        time.sleep(10)
        size = os.path.getsize(LOG)
        print(round(time.time() - start), "log bytes", size, flush=True)
        # The scripts set logs nothing through the long load, so a quiet file isn't the end.
        if size == last and "scripts" not in sys.argv:
            break
        last = size
    print("eip", x32.get("Register/Get", register="eip"), "seconds", round(time.time() - start))


if __name__ == "__main__":
    main()
