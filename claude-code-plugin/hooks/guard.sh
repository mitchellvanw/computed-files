#!/bin/sh
# The computed plugin's Claude Code hooks. One argument, the event:
#
#   session  says once, at session start, when the guard cannot run
#   pre      refuses an Edit, MultiEdit or Write that changes a region's
#            body or closer (PreToolUse)
#   post     adds the `check` lines of a file left with stale regions
#            (PostToolUse)
#
# `computed guard --hook` reads the hook's JSON on stdin and prints the
# decision, so this script needs nothing but computed itself. Whatever
# happens here, the exit is 0: a hook that cannot answer allows the edit and
# says so, it never blocks one by failing.

# The messages quote commands in backticks, which are not meant to expand.
# shellcheck disable=SC2016

event=$1

if ! command -v computed >/dev/null 2>&1; then
  if [ "$event" = session ]; then
    printf '%s\n' '{"systemMessage":"computed is not on PATH, so its plugin cannot guard computed regions in this session. Install it with `cargo install computed`, or run /computed-setup."}'
  fi
  exit 0
fi

if [ "$event" = session ]; then
  if ! computed guard --help >/dev/null 2>&1; then
    printf '%s\n' '{"systemMessage":"The computed on PATH has no `guard` command, so its plugin cannot guard computed regions in this session. Upgrade computed."}'
  fi
  exit 0
fi

out=$(computed guard --hook "$event")
status=$?
if [ "$status" -ne 0 ]; then
  printf '{"systemMessage":"computed guard --hook %s exited %s; this edit was not guarded."}\n' "$event" "$status"
  exit 0
fi
if [ -n "$out" ]; then
  printf '%s\n' "$out"
fi
exit 0
