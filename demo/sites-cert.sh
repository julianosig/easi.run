#!/bin/sh
# Makes the demo pages' certificate, once, where demo/compose.yaml mounts it: ~/.config/easi-run/demo-sites/
#   cert.pem  self-signed, P-256, for every demo host (nginx.conf serves them all with it)
#   key.pem   its private key
#   spki      the base64 SHA-256 of its public key, which a demo Chrome accepts though no authority signed it
# Run it from anywhere before the first `docker compose -f demo/compose.yaml up`. A new certificate needs the sites
# and the controller restarted (the controller reads the SPKI when it starts).
set -eu

dir="$HOME/.config/easi-run/demo-sites"
# The demo hostnames are listed in demo/compose.yaml (ADMIN_DEMO_SITES), demo/nginx.conf, demo/sites-cert.sh and
# client/src/ui/demos.ts (and each has a directory under demo/sites/): scripts/demo.py checks that they agree.
hosts="takeover drag upload download sound typing clipboard"

# Docker creates a missing bind-mount source as an empty root-owned directory, so a `compose up` before this script
# leaves directories where the files belong.
for f in cert.pem key.pem spki; do
    if [ -e "$dir/$f" ] && [ ! -f "$dir/$f" ]; then
        echo "$dir/$f exists but is not a regular file (Docker makes a directory there when compose starts first);" \
            "delete it (sudo rm -r) and run this again." >&2
        exit 1
    fi
done
if [ -s "$dir/cert.pem" ] && [ -s "$dir/key.pem" ] && [ -s "$dir/spki" ]; then
    echo "$dir already holds the demo certificate; delete it to make a new one."
    exit 0
fi
if [ -e "$dir/cert.pem" ] || [ -e "$dir/key.pem" ] || [ -e "$dir/spki" ]; then
    echo "$dir holds only part of a demo certificate; delete it and run this again." >&2
    exit 1
fi

umask 077
mkdir -p "$dir"
chmod 0700 "$dir"
san=$(for h in $hosts; do printf 'DNS:%s.play,' "$h"; done)
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -noenc -days 825 -subj /CN=takeover.play \
    -addext "subjectAltName=${san%,}" -keyout "$dir/key.pem" -out "$dir/cert.pem"
openssl pkey -in "$dir/key.pem" -pubout -outform DER | openssl dgst -sha256 -binary | openssl base64 -A > "$dir/spki"
# The sites container's nginx runs as its own uid and reads these through bind mounts; the directory (0700) keeps
# every other host user out.
chmod 0644 "$dir/cert.pem" "$dir/key.pem" "$dir/spki"
echo "Made the demo certificate in $dir (SPKI sha256/$(cat "$dir/spki"))."
