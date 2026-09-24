#!/bin/sh
# Emit an install block for docs/index.html, with the version taken from
# Cargo.toml, so the site cannot advertise a release the crate does not have.
# `hero` (the default) is the block at the top of the page, `start` the one
# under "Get started". The `install` and `start` regions render these.
set -eu
v=$(sed -n 's/^version = "\(.*\)"$/\1/p' ../Cargo.toml | head -1)
case "${1:-hero}" in
hero)
  cat <<HTML
<pre class="mt-6 rounded border border-line bg-ground-2 p-4 text-[13px] leading-relaxed overflow-x-auto m-0"><code>cargo install computed   <span class="text-ink-2"># v$v</span>
computed run             <span class="text-ink-2"># fill what moved</span>
computed check           <span class="text-ink-2"># exit 1 on drift</span></code></pre>
HTML
  ;;
start)
  cat <<HTML
<pre class="rounded border border-line bg-ground-2 p-4 text-[13px] leading-relaxed overflow-x-auto m-0"><code>cargo install computed   <span class="text-ink-2"># v$v</span>
computed run</code></pre>
HTML
  ;;
*)
  echo "usage: site-install.sh [hero|start]" >&2
  exit 2
  ;;
esac
