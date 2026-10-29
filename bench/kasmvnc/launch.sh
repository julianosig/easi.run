#!/bin/sh
# The baseline: Xkasmvnc with the settings the decision measured (60 fps cap, dynamic quality 7-9, all rect threads),
# input accepted for the typing fixture, and HTTPS as KasmVNC is normally served (its client needs a secure context);
# the same Google Chrome binary and flags as easi.run's browser, on X11.
set -eu
: "${BROWSER_URL:?}" "${BROWSER_PROXY:?}"
export HOME=/home/browser
mkdir -p /tmp/.X11-unix && chmod 1777 /tmp/.X11-unix
Xkasmvnc :1 -publicIP 127.0.0.1 -geometry 1280x800 -depth 24 -nolisten tcp -interface 0.0.0.0 -websocketPort 6901 \
    -DisableBasicAuth 1 -SecurityTypes None -httpd /usr/share/kasmvnc/www \
    -sslOnly 1 -cert /etc/ssl/certs/ssl-cert-snakeoil.pem -key /etc/ssl/private/ssl-cert-snakeoil.key \
    -FrameRate 60 -RectThreads 0 -DynamicQualityMin 7 -DynamicQualityMax 9 \
    -AcceptSetDesktopSize 0 -AlwaysShared 1 -DisconnectClients 0 &
# The socket exists a moment before the server answers on it: wait, at most ten seconds, until it answers an X11
# connection setup (Perl is what the image has to speak it with) with success, as Chrome's own connection would be.
# shellcheck disable=SC2016 # Perl's variables, not the shell's
answers='alarm 1; my $s = IO::Socket::UNIX->new(Peer => "/tmp/.X11-unix/X1") or exit 1; my $r = "";
    syswrite($s, pack("a1 x v4 x2", "l", 11, 0, 0, 0)) == 12 or exit 1; sysread($s, $r, 1); $r eq "\1" or exit 1'
deadline=$(($(date +%s) + 10))
until perl -MIO::Socket::UNIX -e "$answers" 2>/dev/null; do
    [ "$(date +%s)" -lt "$deadline" ] || { echo "Xkasmvnc did not answer on :1 within 10 s" >&2; exit 1; }
    sleep 0.1
done
export DISPLAY=:1
# The fixture host's certificate is trusted by its SPKI only, exactly as easi.run's browser trusts it.
spki=$(cat /run/bench/spki)
# easi.run's Google Chrome binary with the flags its browser shares (browser/launch/src/chrome-flags, one per line, a
# line starting with # a comment): the Ozone platform, the proxy address, and the window's position and size (X11
# without a window manager needs them) differ, and there is no launcher, so no DevTools pipe and no network namespace.
# Kiosk mode, as easi.run's benchmark sessions run.
set -f
# shellcheck disable=SC2046 # one word per flag
set -- $(grep -v '^#' /usr/local/share/baseline/chrome-flags)
exec /opt/google/chrome/chrome --ozone-platform=x11 --kiosk "$@" \
    --proxy-server="$BROWSER_PROXY" --ignore-certificate-errors-spki-list="$spki" \
    --window-position=0,0 --window-size=1280,800 -- "$BROWSER_URL"
