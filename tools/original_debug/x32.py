#!/usr/bin/env python3
"""Drives x32dbg's MCPx64dbg HTTP plugin (127.0.0.1:8888). See docs/original-debug.md.

x32.py cmd 'bp 0x44F2D9'      one command
x32.py eip                    where it is, and whether it's running
"""
import json
import sys
import time
import urllib.parse
import urllib.request

BASE = "http://127.0.0.1:8888/"


def get(endpoint, timeout=20, **params):
    url = BASE + endpoint + ("?" + urllib.parse.urlencode(params) if params else "")
    with urllib.request.urlopen(url, timeout=timeout) as response:
        return response.read().decode("utf-8", "replace").strip()


def cmd(text, timeout=20):
    return get("ExecCommand", timeout=timeout, cmd=text)


def running():
    return json.loads(get("IsDebugActive")).get("isRunning")


def wait_stop(seconds, poll=0.2):
    """Seconds until the debuggee stops, or None."""
    start = time.time()
    while time.time() - start < seconds:
        if not running():
            return time.time() - start
        time.sleep(poll)
    return None


if __name__ == "__main__":
    if sys.argv[1] == "cmd":
        print(cmd(sys.argv[2]))
    elif sys.argv[1] == "eip":
        print(get("Register/Get", register="eip"), "running", running())
