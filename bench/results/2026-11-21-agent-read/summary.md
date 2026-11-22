# Agent page reads

Measured 2026-11-21 on x86_64, 96 CPUs, Linux 6.12.63+deb13-amd64: Cua Driver 0.56.9, its release's binary (SHA-256 8bf8d8621042) and the pinned one the sessions' agent image (sha256:38d74794d2f8) carries (SHA-256 26c0f9dd45b5), each under the agent's limits; 4 turns of 3 timed reads for each driver and page.

| posts | elements | release, first piece: median (p95) | patched, first piece: median (p95) | through the admin | speedup | whole page: release, patched | same whole page | failed reads |
|---|---|---|---|---|---|---|---|---|
| 50 | 1162 | 4.32 s (4.39 s) | 0.27 s (0.30 s) | 0.30 s (0.38 s) | 16.1x | 4.43 s, 0.42 s | yes (4 pieces) | 0 |
| 100 | 2312 | 8.85 s (9.03 s) | 0.57 s (0.65 s) | 0.57 s (0.59 s) | 15.5x | 9.14 s, 0.97 s | yes (8 pieces) | 0 |
| 200 | 4612 | 17.90 s (18.02 s) | 1.24 s (1.51 s) | 1.15 s (1.24 s) | 14.4x | 19.36 s, 2.42 s | yes (16 pieces) | 0 |
| 400 | 9212 | 36.39 s (37.16 s) | 2.38 s (2.84 s) | 2.43 s (2.54 s) | 15.3x | 40.57 s, 6.82 s | yes (31 pieces) | 0 |
