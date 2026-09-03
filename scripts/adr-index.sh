#!/bin/sh
# Emit the ADR titles as a linked list, in file order. Used by the
# `adrs` region in CLAUDE.md.
set -eu
for f in docs/adr/*.md; do
  title=$(grep -m1 '^# ' "$f" | cut -c3-)
  printf -- '- [%s](%s)\n' "$title" "$f"
done
