# Read one file's patch from the GitHub pull request files API and print the
# new side's line range of every hunk as a JSON pair, `[first,last]`: the
# lines a review comment may be placed on.

/^@@ / {
  split($3, n, ",")
  first = substr(n[1], 2) + 0
  count = (2 in n) ? n[2] + 0 : 1
  if (count > 0) printf "[%d,%d]\n", first, first + count - 1
}
