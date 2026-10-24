# Publishing easi.run

The landing and the docs are static. `easi-run-site` (nginx, unprivileged, read-only root) serves the release that
`~/.local/share/easi-run/releases/current` points to; `easi-run-tunnel` (cloudflared) connects it to Cloudflare. Both run
on the `easi-run` Docker network with no host port. The site container mounts, read-only:

- `~/.local/share/easi-run/releases` at `/srv` (the releases, `current`, and the shared asset pool);
- `site/deploy/nginx.conf` at `/etc/nginx/nginx.conf`;
- `site/deploy/headers.conf` at `/etc/nginx/easi/headers.conf`.

Commands:

- Publish: `site/deploy/release.sh` (refuses uncommitted changes under `site/`; `npm ci`, build, new release with the
  commit in `REVISION`, atomic switch; keeps five).
- Roll back: `site/deploy/release.sh rollback`.
- Headers or nginx changes: edit `headers.conf` / `nginx.conf`, then `docker restart easi-run-site`. Each is a
  single-file bind mount, so an edit that replaces the file reaches the container only on a restart, not on a reload.

Cloudflare: tunnel `easi-run` (remotely managed) routes, everything else to 404:

- `easi.run`: `/demo/`, `/api/`, `/ws/` and `/sdk/` to the demo's admin (`demo/compose.yaml`, over the
  `easi-run-demo-origin` network), the rest to `http://easi-run-site:8080` (the landing);
- `www.easi.run` to `http://easi-run-site:8080`, which redirects to `https://easi.run`;
- `docs.easi.run` to `http://easi-run-site:8080` (the docs page, from the same release and asset pool).

Each name is a proxied CNAME to the tunnel; the zone enforces HTTPS and TLS 1.2+. The tunnel token lives outside the
repository in `~/.config/easi-run/tunnel.token` (0600); the created tunnel and DNS record IDs are in
`~/.config/easi-run/state`.
