#!/bin/sh
# The action's second step. INPUT_COMMAND picks what it does:
#
#   check    `computed check`, with an error annotation on every region that
#            is not fresh. Exits as computed does.
#   suggest  `computed run --dry-run`. On a pull request, every change block
#            that lies on lines the pull request touched becomes a review
#            comment holding a GitHub suggestion; the files with blocks
#            elsewhere go into one summary comment with their diff, edited in
#            place on later pushes. The diff always goes to the job log and
#            the step summary. Exits as computed does, so drift fails the job.
#
# Environment: INPUT_COMMAND, INPUT_PATHS (space-separated), INPUT_TRUST
# (`true` passes --trust to run), INPUT_ALLOW (space-separated url
# prefixes, each passed to run as --allow), GH_TOKEN, and the runner's
# GITHUB_* variables. Needs computed and jq, and gh for the comments.
set -eu
# INPUT_PATHS and INPUT_ALLOW are split on spaces and never globbed.
set -f

here=$(cd "$(dirname "$0")" && pwd)
command=${INPUT_COMMAND:-check}
paths=${INPUT_PATHS:-}
summary=${GITHUB_STEP_SUMMARY:-/dev/null}
marker='<!-- computed-action:suggest -->'
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# The shortest backtick fence that no line of the file can close.
fence() {
  awk '{
    while (match($0, /`+/)) {
      if (RLENGTH > m) m = RLENGTH
      $0 = substr($0, RSTART + RLENGTH)
    }
  } END {
    n = m < 3 ? 3 : m + 1
    for (i = 0; i < n; i++) s = s "`"
    print s
  }' "$1"
}

case $command in
  check)
    set +e
    # shellcheck disable=SC2086
    computed check $paths
    status=$?
    # shellcheck disable=SC2086
    computed --format json check $paths >"$work/report.json"
    set -e
    jq -r '
      def msg: gsub("%"; "%25") | gsub("\r"; "%0D") | gsub("\n"; "%0A");
      def prop: msg | gsub(":"; "%3A") | gsub(","; "%2C");
      .files[] | (.path | ltrimstr("./")) as $p
      | ( .error // empty
          | "::error file=\($p | prop),line=\(.line // 1),title=computed::\(.message | msg)" ),
        ( .regions[] | select(.state != "fresh" and .state != "volatile")
          | "::error file=\($p | prop),line=\(.line),title=computed \(.state)::"
            + "\(.name // "\(.loader)@\(.line)") is \(.state). Run `computed run` and commit the result."
            + (if .message then "%0A" + (.message | msg) else "" end) )
    ' "$work/report.json"
    exit "$status"
    ;;
  suggest) ;;
  *)
    echo "::error title=computed::command is check or suggest, not $command"
    exit 2
    ;;
esac

trust=
if [ "${INPUT_TRUST:-false}" = true ]; then
  trust=--trust
fi
allow=
for prefix in ${INPUT_ALLOW:-}; do
  allow="$allow --allow $prefix"
done
set +e
# shellcheck disable=SC2086
computed --format json run --dry-run $trust $allow $paths >"$work/run.json"
status=$?
set -e

# The report and the diff, for the log.
jq -r '
  .files[] | .path as $p
  | ( .error // empty | "\($p):\(.line // "") \(.message)" ),
    ( .regions[] | select(.action != "fresh")
      | "\($p):\(.line) \(.name // "") \(.loader) \(.state) \(.action)"
        + (if .message then "\n    " + (.message | gsub("\n"; "\n    ")) else "" end) )
' "$work/run.json" >&2
jq -r '.files[] | .diff // empty' "$work/run.json" >"$work/all.diff"
cat "$work/all.diff"
if [ -s "$work/all.diff" ]; then
  f=$(fence "$work/all.diff")
  {
    echo "### computed: regions that would change"
    echo
    echo "${f}diff"
    cat "$work/all.diff"
    echo "$f"
  } >>"$summary"
fi

case ${GITHUB_EVENT_NAME:-} in
  pull_request | pull_request_target) ;;
  *) exit "$status" ;;
esac
pr=$(jq -r .pull_request.number "$GITHUB_EVENT_PATH")
head=$(jq -r .pull_request.head.sha "$GITHUB_EVENT_PATH")
api="repos/$GITHUB_REPOSITORY"

# A suggestion is a review comment on the head commit, so it needs the
# checkout to be that commit, and GitHub takes one only on lines of the
# pull request's own diff.
inline=true
if [ "$(git rev-parse HEAD 2>/dev/null || true)" != "$head" ]; then
  inline=false
  if [ -s "$work/all.diff" ]; then
    echo "::notice title=computed::the checkout is not the pull request's head commit, so no inline suggestions; check out ref: github.event.pull_request.head.sha for them"
  fi
fi
if $inline && ! gh api --paginate "$api/pulls/$pr/files" >"$work/files.json"; then
  echo "::warning title=computed::could not list the pull request's files; no inline suggestions"
  inline=false
fi

: >"$work/blocks.jsonl"
jq -r '.files[] | select(.diff) | .path' "$work/run.json" | while IFS= read -r path; do
  rel=${path#./}
  jq -r --arg p "$path" '.files[] | select(.path == $p) | .diff' "$work/run.json" |
    awk -f "$here/blocks.awk" >"$work/blocks"
  if $inline; then
    jq -rs --arg f "$rel" 'add | .[] | select(.filename == $f) | .patch // ""' "$work/files.json" |
      awk -f "$here/ranges.awk" | jq -sc . >"$work/ranges"
  else
    echo '[]' >"$work/ranges"
  fi
  jq -c --arg path "$rel" --slurpfile ranges "$work/ranges" '
    . as $b
    | . + { path: $path,
            inline: any($ranges[0][]; .[0] <= $b.start and $b.end <= .[1]) }
  ' "$work/blocks" >>"$work/blocks.jsonl"
done

# The review: one comment per block on the pull request's lines, less the
# ones an earlier push already posted word for word.
existing='[]'
if [ -s "$work/blocks.jsonl" ]; then
  existing=$(gh api --paginate "$api/pulls/$pr/comments" | jq -cs 'add // [] | map({path, line, body})') || existing='[]'
fi
jq -s --arg head "$head" --arg marker "$marker" --argjson existing "$existing" '
  def fence($t): ([$t | scan("`+") | length] | max // 0) as $m
    | [range(if $m < 3 then 3 else $m + 1 end)] | map("`") | join("");
  map(select(.inline))
  | map(fence(.text) as $f
        | { path, line: .end, side: "RIGHT",
            body: ($marker + "\n`computed run` would write this:\n\n" + $f + "suggestion\n"
                   + (if .text == "" then "" else .text + "\n" end) + $f) }
          + (if .start < .end then { start_line: .start, start_side: "RIGHT" } else {} end))
  | map(. as $c | select($existing | any(.path == $c.path and .line == $c.line and .body == $c.body) | not))
  | if length == 0 then empty
    else { commit_id: $head, event: "COMMENT",
           body: "computed: regions this pull request leaves stale, as suggestions. Apply them, or run `computed run` and commit.",
           comments: . }
    end
' "$work/blocks.jsonl" >"$work/review.json"
if [ -s "$work/review.json" ]; then
  gh api --method POST "$api/pulls/$pr/reviews" --input "$work/review.json" >/dev/null ||
    echo "::warning title=computed::could not post the review; the diff is in the log and the step summary"
fi

# The summary comment: the hunks with a block no suggestion could carry.
# Made once, then edited, and told when there is nothing left.
jq -rs 'map(select(.inline | not) | .path) | unique | .[]' "$work/blocks.jsonl" >"$work/rest"
id=$(gh api --paginate "$api/issues/$pr/comments" --jq ".[] | select(.body | startswith(\"$marker\")) | .id" 2>/dev/null | head -n 1) || id=
if [ -s "$work/rest" ]; then
  : >"$work/rest.diff"
  while IFS= read -r rel; do
    keep=$(jq -r --arg p "$rel" 'select(.path == $p and (.inline | not)) | .hunk' "$work/blocks.jsonl" | tr '\n' ' ')
    jq -r --arg a "$rel" --arg b "./$rel" '.files[] | select(.path == $a or .path == $b) | .diff' "$work/run.json" |
      awk -v keep=" $keep" -f "$here/hunks.awk" >>"$work/rest.diff"
  done <"$work/rest"
  f=$(fence "$work/rest.diff")
  {
    echo "$marker"
    echo "### computed: regions that would change"
    echo
    echo "These changes fall outside the lines this pull request touched, so they cannot be suggestions. Run \`computed run\` and commit the result, which writes:"
    echo
    echo "${f}diff"
    head -c 60000 "$work/rest.diff"
    echo "$f"
  } >"$work/body.md"
elif [ -n "$id" ]; then
  if [ "$status" -eq 0 ]; then
    printf '%s\ncomputed: every region is fresh.\n' "$marker" >"$work/body.md"
  else
    printf '%s\ncomputed: every change left is in the review suggestions.\n' "$marker" >"$work/body.md"
  fi
fi
if [ -s "$work/body.md" ] 2>/dev/null; then
  jq -n --rawfile body "$work/body.md" '{body: $body}' >"$work/comment.json"
  if [ -n "$id" ]; then
    gh api --method PATCH "$api/issues/comments/$id" --input "$work/comment.json" >/dev/null ||
      echo "::warning title=computed::could not update the summary comment"
  else
    gh api --method POST "$api/issues/$pr/comments" --input "$work/comment.json" >/dev/null ||
      echo "::warning title=computed::could not post the summary comment; the diff is in the log and the step summary"
  fi
fi
exit "$status"
