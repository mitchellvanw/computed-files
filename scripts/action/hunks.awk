# Print one file's unified diff with only the hunks `keep` names: its
# header, then each hunk whose 1-based index is a space-separated word of
# `keep`, as in `awk -v keep=" 1 3 " -f hunks.awk`.

/^@@ / { hunk++ }

!hunk || index(keep " ", " " hunk " ") { print }
