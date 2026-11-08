#!/bin/sh
# Builds ~/.wine-oblivion-capture from ~/.wine-oblivion with the pinned config. See docs/original-debug.md.
# Wine links the prefix's user folders to the real home, so they become real folders before anything is written:
# on 2026-11-06 a first version wrote through Documents and overwrote the user's own INI.
set -eu
repo=$(cd "$(dirname "$0")/../.." && pwd)
prefix="$HOME/.wine-oblivion-capture"
[ ! -e "$prefix" ] || { echo "exists: $prefix" >&2; exit 1; }
mkdir "$prefix"
wine_user=${OBLIVION_WINE_USER:-$(id -un)}
rsync -a --exclude="drive_c/users/$wine_user/Documents/" --exclude="drive_c/users/$wine_user/AppData/Local/Oblivion/" \
    "$HOME/.wine-oblivion/" "$prefix/"
user="$prefix/drive_c/users/$wine_user"
for name in Desktop Downloads Music Pictures Videos; do
    [ -L "${user:?}/${name:?}" ] && rm -- "${user:?}/${name:?}"
    mkdir -p "$user/$name"
done
mkdir -p "$user/Documents/My Games/Oblivion/Saves" "$user/AppData/Local/Oblivion"
target=$(readlink -f "$user/Documents")
case "$target" in "$prefix"/*) ;; *) echo "Documents leaves the prefix: $target" >&2; exit 1;; esac
install -m644 "$repo/fixtures/config/Oblivion.ini" "$repo/fixtures/config/BlendSettings.ini" \
    "$user/Documents/My Games/Oblivion/"
install -m644 "$repo/fixtures/config/Plugins.txt" "$user/AppData/Local/Oblivion/Plugins.txt"
