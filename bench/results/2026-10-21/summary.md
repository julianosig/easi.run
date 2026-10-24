# Benchmark (docs/design.md §8)

Machine: cpu: Intel(R) Xeon(R) Platinum 8160 CPU @ 2.10GHz (96 logical); gpu: none (software rendering on both systems and the client); os: Debian GNU/Linux 13 (trixie) 6.12.63+deb13-amd64; docker: 29.9.0; client: Playwright v1.61.1-noble Chromium; browser: baseline: Google Chrome 156.0.8078.47 (c843efb5e440); prototype: Google Chrome 156.0.8078.47 (da8572161c56); kiosk mode on both; lanes: 3; seed: 0

## Headed client

| System | Fixture | Runs (failed) | Presented fps | kbit/s | Media B/s | Capture-to-draw p50/p95 ms | Input-to-draw p50/p95 ms | CPU cores | fps/CPU | SSIM | screen MiB (sum of peaks) | display MiB |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| prototype | animation | 5 (0) | 27.0 | 2053 | 256564 | 77/153 | –/– | 1.83 | 14.7 | – | 297 (332) | 52.5 |
| prototype | article | 5 (0) | 0.0 | 0 | 0 | –/– | –/– | 0.01 | 0.0 | – | 210 (258) | 15.5 |
| prototype | quality | 5 (0) | 54.2 | 1565 | 195585 | 42/56 | –/– | 0.91 | 59.8 | 1.0000 | 315 (323) | 23.1 |
| prototype | scroll | 5 (0) | 55.3 | 1761 | 220129 | 47/65 | –/– | 1.07 | 52.0 | – | 289 (325) | 66.4 |
| prototype | text-scroll | 5 (0) | 55.9 | 1855 | 231874 | 45/61 | –/– | 1.09 | 51.0 | 0.9894 | 311 (363) | 58.3 |
| prototype | transform | 5 (0) | 42.4 | 1981 | 247610 | 68/115 | –/– | 1.58 | 26.9 | – | 311 (333) | 73.9 |
| prototype | typing | 5 (0) | 0.0 | 127 | 15892 | –/– | 30/40 | 0.13 | 0.0 | – | 223 (257) | 19.1 |
| prototype | video | 5 (1) | 38.4 | 2045 | 255615 | 63/99 | –/– | 1.54 | 25.0 | – | 333 (381) | 61.2 |
| baseline | animation | 5 (0) | 18.2 | 5288 | 660961 | 133/334 | –/– | 1.91 | 9.5 | – | 268 (300) | – |
| baseline | article | 5 (0) | 0.0 | 0 | 0 | –/– | –/– | 0.01 | 0.0 | – | 207 (248) | – |
| baseline | quality | 5 (0) | 23.7 | 1740 | 217545 | 67/83 | –/– | 0.81 | 29.3 | 0.2302 | 310 (312) | – |
| baseline | scroll | 5 (0) | 24.3 | 12448 | 1556057 | 67/88 | –/– | 0.93 | 26.2 | – | 235 (271) | – |
| baseline | text-scroll | 5 (0) | 24.4 | 16420 | 2052545 | 83/87 | –/– | 1.13 | 21.6 | 0.2864 | 272 (307) | – |
| baseline | transform | 5 (0) | 24.3 | 25427 | 3178404 | 100/118 | –/– | 1.47 | 16.5 | – | 265 (277) | – |
| baseline | typing | 5 (0) | 0.0 | 141 | 17679 | –/– | –/– | 0.13 | 0.0 | – | 218 (241) | – |
| baseline | video | 5 (0) | 24.3 | 18167 | 2270816 | 101/125 | –/– | 1.78 | 13.7 | – | 283 (320) | – |

## Headless client

| System | Fixture | Runs (failed) | Presented fps | kbit/s | Media B/s | Capture-to-draw p50/p95 ms | Input-to-draw p50/p95 ms | CPU cores | fps/CPU | SSIM | screen MiB (sum of peaks) | display MiB |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| prototype | animation | 5 (0) | 25.5 | 2186 | 273162 | 78/152 | –/– | 1.80 | 14.2 | – | 297 (331) | 52.6 |
| prototype | article | 5 (0) | 0.0 | 0 | 0 | –/– | –/– | 0.01 | 0.0 | – | 211 (259) | 15.5 |
| prototype | quality | 5 (0) | 53.5 | 1564 | 195504 | 40/54 | –/– | 0.92 | 57.8 | 1.0000 | 314 (319) | 23.0 |
| prototype | scroll | 5 (0) | 54.9 | 1751 | 218857 | 47/65 | –/– | 1.03 | 52.8 | – | 293 (325) | 66.9 |
| prototype | text-scroll | 5 (0) | 55.2 | 1862 | 232696 | 43/60 | –/– | 1.08 | 50.7 | 0.9896 | 312 (363) | 57.9 |
| prototype | transform | 5 (1) | 37.7 | 1957 | 244654 | 68/114 | –/– | 1.56 | 24.2 | – | 308 (330) | 70.5 |
| prototype | typing | 5 (0) | 0.0 | 128 | 15939 | –/– | 28/37 | 0.12 | 0.0 | – | 223 (257) | 19.1 |
| prototype | video | 5 (0) | 36.2 | 2068 | 258498 | 63/100 | –/– | 1.51 | 23.8 | – | 323 (376) | 61.1 |
| baseline | animation | 5 (0) | 18.5 | 5202 | 650293 | 132/333 | –/– | 1.90 | 9.7 | – | 267 (300) | – |
| baseline | article | 5 (0) | 0.0 | 0 | 0 | –/– | –/– | 0.01 | 0.0 | – | 209 (247) | – |
| baseline | quality | 5 (0) | 24.3 | 1740 | 217562 | 67/84 | –/– | 0.82 | 29.3 | 0.9927 | 309 (312) | – |
| baseline | scroll | 5 (0) | 24.4 | 12441 | 1555064 | 83/100 | –/– | 0.90 | 26.8 | – | 236 (271) | – |
| baseline | text-scroll | 5 (0) | 24.3 | 16433 | 2054104 | 83/87 | –/– | 1.13 | 21.5 | 0.9767 | 272 (306) | – |
| baseline | transform | 5 (0) | 24.4 | 25425 | 3178141 | 100/116 | –/– | 1.47 | 16.5 | – | 265 (277) | – |
| baseline | typing | 5 (0) | 0.0 | 142 | 17697 | –/– | 69/82 | 0.13 | 0.0 | – | 217 (240) | – |
| baseline | video | 5 (0) | 24.4 | 18130 | 2266201 | 83/108 | –/– | 1.77 | 13.8 | – | 281 (317) | – |
