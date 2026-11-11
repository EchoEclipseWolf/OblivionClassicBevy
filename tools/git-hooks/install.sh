#!/bin/sh
# Installs the pre-commit shim into the repository's shared hooks folder (beside the Git LFS hooks, which stay).
# Run once per clone; worktrees share it. The shim runs the worktree's own tools/git-hooks/pre-commit.
hooks=$(git rev-parse --git-common-dir)/hooks
target=$hooks/pre-commit
if [ -e "$target" ] && ! grep -q 'tools/git-hooks/pre-commit' "$target"; then
    echo "install.sh: $target exists and isn't this shim; not overwriting it" >&2
    exit 1
fi
cat > "$target" <<'SHIM'
#!/bin/sh
hook=$(git rev-parse --show-toplevel)/tools/git-hooks/pre-commit
[ -x "$hook" ] || exit 0
exec "$hook" "$@"
SHIM
chmod +x "$target"
echo "installed $target"
