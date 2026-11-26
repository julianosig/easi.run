# Agent page reads

Measured 2026-11-25 on x86_64, 96 CPUs, Linux 6.12.63+deb13-amd64: Cua Driver 0.62.1, its release's binary (SHA-256 7f8ba9e521c1) and the pinned one the sessions' agent image (sha256:5975ebcdad31) carries (SHA-256 d75a8d008467), each under the agent's limits; 4 turns of 3 timed reads for each driver and page.

| posts | elements | release, first piece: median (p95) | patched, first piece: median (p95) | through the admin | speedup | whole page: release, patched | same whole page | failed reads |
|---|---|---|---|---|---|---|---|---|
| 50 | 1162 | 4.40 s (4.45 s) | 0.27 s (0.35 s) | 0.31 s (0.36 s) | 16.4x | 4.54 s, 0.41 s | yes (4 pieces) | 0 |
| 100 | 2312 | 8.81 s (8.99 s) | 0.53 s (0.59 s) | 0.59 s (0.59 s) | 16.5x | 9.24 s, 0.96 s | yes (8 pieces) | 0 |
| 200 | 4612 | 17.93 s (18.24 s) | 1.26 s (1.40 s) | 1.08 s (1.24 s) | 14.2x | 19.30 s, 2.45 s | yes (16 pieces) | 0 |
| 400 | 9212 | 37.14 s (38.60 s) | 2.50 s (2.80 s) | 2.47 s (2.54 s) | 14.8x | 42.10 s, 6.97 s | yes (31 pieces) | 0 |
