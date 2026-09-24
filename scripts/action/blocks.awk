# Read one file's unified diff, as `computed run --dry-run` prints it, and
# print one JSON object per change block, a run of `-` and `+` lines:
#
#   {"hunk":H,"start":S,"end":E,"text":"..."}
#
# H counts the hunks from 1. S..E are the old side's lines the block
# replaces and `text` is what replaces them, which is what a GitHub
# suggestion on lines S..E holds. A block that only inserts is anchored on
# the context line before it, or, at the top of a hunk, the one after, so it
# still has a line to replace.

function esc(s) {
  gsub(/\\/, "\\\\", s)
  gsub(/"/, "\\\"", s)
  gsub(/\t/, "\\t", s)
  gsub(/\r/, "\\r", s)
  gsub(/\n/, "\\n", s)
  return s
}

function emit(start, end, text) {
  printf "{\"hunk\":%d,\"start\":%d,\"end\":%d,\"text\":\"%s\"}\n", hunk, start, end, esc(text)
}

function add(line) {
  plus = nplus ? plus "\n" line : line
  nplus++
}

function flush() {
  if (!inblock) return
  inblock = 0
  if (nminus > 0) {
    emit(bstart, bstart + nminus - 1, plus)
  } else if (havectx) {
    emit(ctxline, ctxline, nplus ? ctx "\n" plus : ctx)
  } else {
    pending = 1
    return
  }
  plus = ""
  nplus = 0
}

/^@@ / {
  flush()
  split($2, o, ",")
  old = substr(o[1], 2) + 0
  inhunk = 1
  hunk++
  havectx = 0
  next
}

!inhunk { next }

/^\\/ { next }

/^ / {
  flush()
  line = substr($0, 2)
  if (pending) {
    emit(old, old, nplus ? plus "\n" line : line)
    pending = 0
    plus = ""
    nplus = 0
  }
  ctx = line
  ctxline = old
  havectx = 1
  old++
  next
}

/^-/ {
  if (!inblock) { inblock = 1; bstart = old; nminus = 0; plus = ""; nplus = 0 }
  nminus++
  old++
  next
}

/^\+/ {
  if (!inblock) { inblock = 1; bstart = old; nminus = 0; plus = ""; nplus = 0 }
  add(substr($0, 2))
  next
}

END { flush() }
