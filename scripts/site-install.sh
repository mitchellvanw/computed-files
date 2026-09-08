#!/bin/sh
# Emit the install block for the hero of docs/index.html, with the version
# taken from Cargo.toml. The `install` region on that page renders this, so
# the site cannot advertise a release the crate does not have.
set -eu
v=$(sed -n 's/^version = "\(.*\)"$/\1/p' ../Cargo.toml | head -1)
cat <<HTML
<pre class="mt-6 rounded border border-line bg-ground-2 p-4 text-[13px] leading-relaxed overflow-x-auto m-0"><code>cargo install computed   <span class="text-ink-2"># v$v</span>
computed run             <span class="text-ink-2"># fill what moved</span>
computed check           <span class="text-ink-2"># exit 1 on drift</span></code></pre>
HTML
