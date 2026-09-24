#!/bin/sh
# Emit one synopsis line per command, the commands, arguments and flags all
# taken from the binary's own help. Used by the `commands` region in
# claude-code-plugin/skills/REFERENCE.md, so a change to the command line shows up there.
# The binary is this checkout's, built if need be, not whatever is on PATH.
set -eu
cargo build --quiet --manifest-path "$COMPUTED_ROOT/Cargo.toml"
computed="$COMPUTED_ROOT/target/debug/computed"
commands=$("$computed" --help |
  sed -n '/^Commands:/,/^$/s/^  \([a-z][a-z-]*\) .*/\1/p' |
  grep -v -x help)
for c in $commands; do
  help=$("$computed" "$c" --help)
  args=$(printf '%s\n' "$help" |
    sed -n "s/^Usage: computed $c //p" |
    sed -e 's/\[OPTIONS\] *//' -e 's/\.\.\.//g' |
    tr 'A-Z' 'a-z')
  flags=$(printf '%s\n' "$help" |
    sed -n 's/^ *\(-[a-zA-Z], \)*\(--[a-z-]*\)\( <\([A-Z]*\)>\)*.*/[\2 \4]/p' |
    sed 's/ \]/]/' |
    grep -v -e '--help' -e '--verbose' -e '--format' |
    tr '\n' ' ')
  printf 'computed %-8s %s\n' "$c" "$(printf '%s %s' "$args" "$flags" | tr -s ' ')" |
    sed -e 's/ *$//'
done
