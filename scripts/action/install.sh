#!/bin/sh
# Install the computed release binary for this runner and put it on the
# PATH: the action's first step. The archive is checked against the
# release's SHA256SUMS before it is unpacked.
#
# Environment: INPUT_VERSION (`latest`, a tag such as `v0.2.0` or `0.2.0`, or
# `installed` to use the computed already on PATH), GH_TOKEN for `gh`,
# COMPUTED_REPO (default mitchellvanw/computed-files), and the runner's
# RUNNER_TEMP, GITHUB_PATH and GITHUB_OUTPUT.
set -eu

repo=${COMPUTED_REPO:-mitchellvanw/computed-files}
version=${INPUT_VERSION:-latest}

if [ "$version" = installed ]; then
  if ! command -v computed >/dev/null 2>&1; then
    echo "::error title=computed::version: installed, but computed is not on PATH"
    exit 1
  fi
  installed=$(computed --version)
  echo "Using $installed from $(command -v computed)"
  echo "version=${installed#computed }" >>"${GITHUB_OUTPUT:-/dev/null}"
  exit 0
fi

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target=x86_64-unknown-linux-musl ;;
  Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-musl ;;
  Darwin-arm64) target=aarch64-apple-darwin ;;
  Darwin-x86_64) target=x86_64-apple-darwin ;;
  *)
    echo "::error title=computed::no release binary for $(uname -s) $(uname -m); build it with cargo install computed and set version: installed"
    exit 1
    ;;
esac

case $version in
  latest) tag=$(gh release view --repo "$repo" --json tagName --jq .tagName) ;;
  v*) tag=$version ;;
  *) tag=v$version ;;
esac

name="computed-$tag-$target"
dir="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/computed-$tag"
mkdir -p "$dir"
gh release download "$tag" --repo "$repo" --dir "$dir" --clobber \
  --pattern "$name.tar.gz" --pattern SHA256SUMS

# SHA256SUMS holds one `<sum>  <archive>` line per target.
if ! grep " $name.tar.gz\$" "$dir/SHA256SUMS" >"$dir/$name.sha256"; then
  echo "::error title=computed::SHA256SUMS of $tag has no entry for $name.tar.gz"
  exit 1
fi
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$dir" && sha256sum -c "$name.sha256")
else
  (cd "$dir" && shasum -a 256 -c "$name.sha256")
fi

tar -xzf "$dir/$name.tar.gz" -C "$dir"
echo "$dir/$name" >>"${GITHUB_PATH:-/dev/null}"
"$dir/$name/computed" --version
echo "version=${tag#v}" >>"${GITHUB_OUTPUT:-/dev/null}"
