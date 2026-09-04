#!/bin/sh
# Emit one synopsis line per command, arguments and flags taken from the
# binary's own help. Used by the `commands` region in
# skills/computed-setup/REFERENCE.md, so a change to the command line shows up there.
set -eu
for c in run check clean trust untrust; do
  help=$(computed "$c" --help)
  args=$(printf '%s\n' "$help" |
    sed -n "s/^Usage: computed $c //p" |
    sed -e 's/\[OPTIONS\] *//' -e 's/\.\.\.//g' |
    tr 'A-Z' 'a-z')
  flags=$(printf '%s\n' "$help" |
    sed -n 's/^ *\(-[a-zA-Z], \)*\(--[a-z-]*\).*/[\2]/p' |
    grep -v -e '--help' -e '--verbose' |
    tr '\n' ' ')
  printf 'computed %-8s %s\n' "$c" "$(printf '%s %s' "$args" "$flags" | tr -s ' ')" |
    sed -e 's/ *$//'
done
