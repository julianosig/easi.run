# shellcheck shell=sh
# Sourced by scripts/svt-av1.sh and scripts/opus.sh, which set NAME, VERSION, SHA256, URL, PREFIX, LIBRARY, DOC and
# NOTICES: static_library builds NAME VERSION from its release source archive at URL, whose bytes must have SHA256, as
# the static library PREFIX/lib/LIBRARY, with CMake and the arguments it is given, unless PREFIX already holds that
# version. NOTICES are the archive's license, patent and notice texts, which every binary linking it ships (the
# display image, in /usr/share/doc/easi.run/DOC/), installed in PREFIX/share/doc/DOC.
static_library() {
    if [ -f "$PREFIX/lib/$LIBRARY" ] && [ -f "$PREFIX/share/doc/$DOC/${NOTICES%% *}" ] &&
        [ "$(cat "$PREFIX/.version" 2>/dev/null)" = "$VERSION" ]; then
        echo "$NAME $VERSION is already in $PREFIX"
        return 0
    fi
    WORK=$(mktemp -d)
    trap 'rm -rf "$WORK"' EXIT
    curl -fsSL --retry 5 --retry-all-errors -o "$WORK/source.tar.gz" "$URL"
    echo "$SHA256  $WORK/source.tar.gz" | sha256sum -c -
    mkdir "$WORK/src"
    tar -xzf "$WORK/source.tar.gz" -C "$WORK/src" --strip-components=1
    cmake -S "$WORK/src" -B "$WORK/build" -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF \
        -DCMAKE_INSTALL_PREFIX="$PREFIX" "$@"
    cmake --build "$WORK/build" -j "$(nproc)"
    cmake --install "$WORK/build"
    for file in $NOTICES; do install -D -m 0644 "$WORK/src/$file" "$PREFIX/share/doc/$DOC/$file"; done
    echo "$VERSION" > "$PREFIX/.version"
}
