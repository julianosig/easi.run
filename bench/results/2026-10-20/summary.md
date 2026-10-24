# Benchmark (docs/design.md §8)

Machine: cpu: Intel(R) Xeon(R) Platinum 8160 CPU @ 2.10GHz (96 logical); gpu: none (software rendering on both systems and the client); os: Debian GNU/Linux 13 (trixie) 6.12.63+deb13-amd64; docker: 29.9.0; client: Playwright v1.61.1-noble Chromium; lanes: 3; seed: 0

## Headed client

| System | Fixture | Runs (failed) | Presented fps | kbit/s | Media B/s | Capture-to-draw p50/p95 ms | Input-to-draw p50/p95 ms | CPU cores | fps/CPU | SSIM | screen MiB (sum of peaks) | display MiB |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| prototype | animation | 5 (0) | 25.2 | 1800 | 224927 | 83/168 | –/– | 1.78 | 14.2 | – | 247 (254) | 52.1 |
| prototype | article | 5 (0) | 0.0 | 0 | 0 | –/– | –/– | 0.01 | 0.0 | – | 163 (175) | 15.3 |
| prototype | quality | 5 (0) | 53.7 | 1571 | 196281 | 42/56 | –/– | 0.96 | 56.1 | 1.0000 | 263 (269) | 18.9 |
| prototype | scroll | 5 (0) | 56.4 | 2133 | 266543 | 47/60 | –/– | 1.01 | 55.6 | – | 238 (242) | 59.9 |
| prototype | text-scroll | 5 (0) | 55.1 | 2065 | 258069 | 46/61 | –/– | 1.15 | 48.0 | 0.9891 | 265 (274) | 61.3 |
| prototype | transform | 5 (0) | 40.4 | 2015 | 251863 | 66/107 | –/– | 1.63 | 24.7 | – | 261 (272) | 72.8 |
| prototype | typing | 5 (0) | 0.0 | 128 | 15927 | –/– | 28/36 | 0.14 | 0.0 | – | 175 (176) | 18.8 |
| prototype | video | 5 (0) | 44.4 | 2287 | 285837 | 59/86 | –/– | 1.61 | 27.9 | – | 270 (277) | 53.3 |
| baseline | animation | 5 (0) | 18.7 | 5363 | 670319 | 133/337 | –/– | 1.93 | 9.6 | – | 229 (238) | – |
| baseline | article | 5 (0) | 0.0 | 0 | 0 | –/– | –/– | 0.00 | 0.0 | – | 172 (175) | – |
| baseline | quality | 5 (0) | 22.8 | 1740 | 217502 | 67/100 | –/– | 0.86 | 26.6 | 0.2303 | 274 (274) | – |
| baseline | scroll | 5 (0) | 24.4 | 12538 | 1567205 | 67/85 | –/– | 0.98 | 24.8 | – | 198 (205) | – |
| baseline | text-scroll | 5 (0) | 24.4 | 16433 | 2054078 | 83/85 | –/– | 1.25 | 19.6 | 0.2769 | 234 (237) | – |
| baseline | transform | 5 (0) | 24.4 | 25741 | 3217573 | 100/117 | –/– | 1.53 | 15.9 | – | 227 (227) | – |
| baseline | typing | 5 (0) | 0.0 | 142 | 17691 | –/– | –/– | 0.14 | 0.0 | – | 180 (181) | – |
| baseline | video | 5 (0) | 24.4 | 18199 | 2274884 | 85/109 | –/– | 1.82 | 13.4 | – | 244 (249) | – |

## Headless client

| System | Fixture | Runs (failed) | Presented fps | kbit/s | Media B/s | Capture-to-draw p50/p95 ms | Input-to-draw p50/p95 ms | CPU cores | fps/CPU | SSIM | screen MiB (sum of peaks) | display MiB |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| prototype | animation | 5 (0) | 25.4 | 1896 | 237020 | 78/151 | –/– | 1.75 | 14.5 | – | 248 (253) | 52.2 |
| prototype | article | 5 (0) | 0.0 | 0 | 0 | –/– | –/– | 0.01 | 0.0 | – | 163 (175) | 15.3 |
| prototype | quality | 5 (0) | 53.1 | 1581 | 197542 | 40/53 | –/– | 0.93 | 57.2 | 1.0000 | 264 (268) | 18.9 |
| prototype | scroll | 5 (0) | 55.1 | 1843 | 230313 | 46/61 | –/– | 0.98 | 56.3 | – | 241 (245) | 61.4 |
| prototype | text-scroll | 5 (1) | 53.1 | 2171 | 271337 | 46/64 | –/– | 1.12 | 47.1 | 0.9886 | 266 (275) | 61.5 |
| prototype | transform | 5 (0) | 36.4 | 1960 | 245000 | 67/112 | –/– | 1.65 | 22.1 | – | 260 (271) | 72.4 |
| prototype | typing | 5 (0) | 0.0 | 128 | 15931 | –/– | 28/36 | 0.14 | 0.0 | – | 176 (177) | 18.7 |
| prototype | video | 5 (0) | 39.2 | 2275 | 284311 | 60/89 | –/– | 1.63 | 24.1 | – | 273 (283) | 54.2 |
| baseline | animation | 5 (0) | 19.0 | 5149 | 643593 | 132/327 | –/– | 1.91 | 10.0 | – | 230 (238) | – |
| baseline | article | 5 (0) | 0.0 | 0 | 0 | –/– | –/– | 0.00 | 0.0 | – | 173 (176) | – |
| baseline | quality | 5 (0) | 22.3 | 1740 | 217544 | 66/100 | –/– | 0.84 | 26.0 | 0.9927 | 269 (270) | – |
| baseline | scroll | 5 (0) | 24.3 | 12534 | 1566770 | 67/88 | –/– | 0.97 | 25.0 | – | 198 (206) | – |
| baseline | text-scroll | 5 (0) | 24.3 | 16454 | 2056720 | 83/86 | –/– | 1.16 | 21.0 | 0.9763 | 234 (237) | – |
| baseline | transform | 5 (0) | 24.4 | 25372 | 3171514 | 100/116 | –/– | 1.51 | 16.2 | – | 225 (225) | – |
| baseline | typing | 5 (0) | 0.0 | 142 | 17701 | –/– | 72/82 | 0.14 | 0.0 | – | 179 (181) | – |
| baseline | video | 5 (0) | 24.3 | 18106 | 2263251 | 86/119 | –/– | 1.85 | 13.1 | – | 246 (252) | – |
