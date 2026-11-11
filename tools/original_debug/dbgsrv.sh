#!/bin/sh
# IDA's remote Windows debug server for the original Oblivion under Wine. Run it inside the virtual display, since
# the game it launches inherits this environment. Port 23947, so it can't meet Evil Genius's 23946.
export WINEPREFIX="$HOME/.wine-oblivion-capture" WINEDEBUG=-all
cd /run/media/natfoth/Echo/OblivionBot || exit 1
exec wine "$HOME/ida-home-9.5/dbgsrv/win32_remote32.exe" -i127.0.0.1 -p23947 "$@"
