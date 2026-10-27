#!/bin/sh
# Turns the workspace copy in DIR into its dependency skeleton, in place: every member keeps its manifest, and each of
# its targets (src/lib.rs, src/main.rs, src/bin/*.rs, build.rs) becomes an empty one; its other sources go. The lock
# file and the patched crates under third_party/ stay whole. The Dockerfiles compile the dependencies from it in a layer
# of their own, which only a manifest or the lock file changes, so a source change rebuilds the workspace's crates
# alone, also on a CI runner that keeps no cargo target directory between runs.
set -eu
dir=$1
cd "$dir"
find . -path ./third_party -prune -o -name '*.rs' -type f -print | while read -r file; do
    case $file in
        */src/lib.rs) : >"$file" ;;
        */src/main.rs | */src/bin/*.rs | */build.rs) echo 'fn main() {}' >"$file" ;;
        *) rm "$file" ;;
    esac
done
# Files the sources read (JSON, text) go too: the skeleton's targets read none.
find . -path ./third_party -prune -o -path '*/src/*' -type f ! -name '*.rs' -exec rm {} +
# The members, as `cargo clean` names them (`-p NAME` each), so a build from the skeleton can drop their empty crates
# and leave only the dependencies' artifacts.
find . -path ./third_party -prune -o -name Cargo.toml ! -path ./Cargo.toml -print | sort | while read -r manifest; do
    sed -n '0,/^name = /s/^name = "\(.*\)"$/-p \1/p' "$manifest"
done >.members
# Only contents count: the layers built from the skeleton stay cached whatever the sources' times were.
find . -exec touch -h -d @0 {} +
