#!/usr/bin/env python3
"""Private, headless KWin/Xwayland session for automated tests (adapted from CreeperWorld3Bevy).

start STATE; exec STATE -- COMMAND...; stop STATE
Never attaches to or replaces the user's compositor. STATE must be a new folder.
"""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time


def start_time(pid):
    return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[19]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["start", "exec", "stop", "capture"])
    parser.add_argument("state", type=Path)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    state = args.state.resolve()
    if args.action == "capture":
        keys = ["DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "XDG_RUNTIME_DIR",
                "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME",
                "DBUS_SESSION_BUS_ADDRESS"]
        environment = {key: os.environ[key] for key in keys if key in os.environ}
        temporary = state / "environment.tmp"
        temporary.write_text(json.dumps(environment, indent=2))
        temporary.replace(state / "environment.json")
        while True:
            signal.pause()
    elif args.action == "start":
        state.mkdir(mode=0o700, parents=True, exist_ok=False)
        env = os.environ.copy()
        for key in ["DISPLAY", "WAYLAND_DISPLAY", "WAYLAND_SOCKET", "XAUTHORITY",
                    "DBUS_SESSION_BUS_ADDRESS", "SESSION_MANAGER"]:
            env.pop(key, None)
        for key, name in [("XDG_RUNTIME_DIR", "runtime"), ("XDG_CONFIG_HOME", "config"),
                          ("XDG_CACHE_HOME", "cache"), ("XDG_DATA_HOME", "data")]:
            path = state / name
            path.mkdir(mode=0o700)
            env[key] = str(path)
        env["QT_QPA_PLATFORM"] = "offscreen"
        session = state / "session.py"
        session.write_text(f"#!{sys.executable}\nimport os\nos.execv({sys.executable!r}, "
                           f"{[sys.executable, str(Path(__file__).resolve()), 'capture', str(state)]!r})\n")
        session.chmod(0o700)
        command = ["dbus-run-session", "--", "kwin_wayland", "--virtual",
                   "--width", os.environ.get("OBLIVION_VIRTUAL_WIDTH", "1920"),
                   "--height", os.environ.get("OBLIVION_VIRTUAL_HEIGHT", "1080"), "--output-count", "1",
                   "--socket", "oblivion-virtual", "--xwayland", "--no-lockscreen",
                   "--no-global-shortcuts", "--no-kactivities",
                   "--exit-with-session", str(session)]
        with (state / "server.log").open("wb") as log:
            process = subprocess.Popen(command, env=env, stdout=log,
                                       stderr=subprocess.STDOUT, start_new_session=True)
        record = {"pid": process.pid, "start_time": start_time(process.pid)}
        (state / "server.json").write_text(json.dumps(record))
        try:
            deadline = time.monotonic() + 30
            while not (state / "environment.json").exists():
                if process.poll() is not None:
                    raise RuntimeError(f"Virtual display exited; see {state / 'server.log'}")
                if time.monotonic() >= deadline:
                    raise RuntimeError(f"Virtual display timed out; see {state / 'server.log'}")
                time.sleep(0.1)
            environment = json.loads((state / "environment.json").read_text())
            if not environment.get("DISPLAY") or environment["DISPLAY"] == os.environ.get("DISPLAY"):
                raise RuntimeError("Virtual X display is missing or matches the desktop")
            print(json.dumps({"state": str(state), "pid": process.pid,
                              "display": environment["DISPLAY"]}))
        except BaseException:
            os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=10)
            raise
    else:
        record = json.loads((state / "server.json").read_text())
        pid = record["pid"]
        try:
            valid = start_time(pid) == record["start_time"]
        except FileNotFoundError:
            valid = False
        if not valid:
            if args.action == "stop":
                return
            raise RuntimeError("Virtual display is not running")
        if args.action == "stop":
            os.killpg(pid, signal.SIGTERM)
            # KWin writes config/ on exit, which would recreate a state folder removed right after stop, and it can
            # outlive the group leader, so wait for the whole process group.
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                try:
                    os.killpg(pid, 0)
                except ProcessLookupError:
                    return
                time.sleep(0.05)
            raise RuntimeError("Virtual display did not exit")
        command = args.command
        if command and command[0] == "--":
            command = command[1:]
        if not command:
            parser.error("exec requires a command after --")
        env = os.environ.copy()
        env.update(json.loads((state / "environment.json").read_text()))
        env.pop("WAYLAND_SOCKET", None)
        env["OBLIVION_VIRTUAL_DISPLAY_STATE"] = str(state)
        raise SystemExit(subprocess.call(command, env=env))


if __name__ == "__main__":
    main()
