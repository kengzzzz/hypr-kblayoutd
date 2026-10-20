# Benchmark Notes

Measured 2026-05-18 in a live Hyprland session: `kb_layout = us,th`, ten keyboards in `hyprctl devices -j`.

Spawning `hyprctl` vs. writing the same command to `.socket.sock`:

| Operation | Mean | p50 | p95 |
| --- | ---: | ---: | ---: |
| `hyprctl getoption input:kb_layout -j` | 1194-1261 us | 1097-1155 us | 1672-1721 us |
| direct `j/getoption input:kb_layout` | 4-5 us | 4-5 us | 6 us |
| `hyprctl switchxkblayout` for one keyboard | 1153-1198 us | 1111-1136 us | 1292-1503 us |
| direct `switchxkblayout` for one keyboard | 5-9 us | 5-9 us | 6-10 us |
| `hyprctl switchxkblayout` for ten keyboard entries | 11429 us | 11425 us | 12335 us |
| direct `switchxkblayout` for ten keyboard entries | 57 us | 58 us | 64 us |

Event parsing and state microbenchmarks:

| Operation | Result |
| --- | --- |
| Upstream-style line parsing with `String::from_utf8_lossy` and `split().collect()` | 1 allocation/event |
| `str::from_utf8`/`read_line` plus `split_once` | 0 allocations/event |
| Formatting window addresses as strings | 2 allocations/event |
| Parsing numeric window addresses | 0 allocations/event |
| 1 million uncontended mutex increments | about 7760 us |
| 1 million plain state increments | about 176 us |
| Default class lookup, 3 classes, linear scan | about 1237 us |
| Default class lookup, 3 classes, `HashMap` | about 7201 us |
| Default class lookup, 200 classes, linear scan | about 72965 us |
| Default class lookup, 200 classes, `HashMap` | about 7162 us |

Decisions:

- Direct IPC: the only change with a large latency win.
- Raw IPC `switchxkblayout` takes no `--` separator: `switchxkblayout <keyboard> <index>`.
- One plain state struct, no global mutexes: simpler, no lock-lifetime bugs.
- Allocation-free parsing and numeric window addresses on the hot path.
- Class defaults in a `HashMap`: not needed for speed at small sizes, but scales.
- No more micro-optimizations without a benchmark showing a bottleneck.

Raw IPC request formats:

| Request | Result |
| --- | --- |
| `switchxkblayout -- keychron-keychron-k2 0` | `device not found` |
| `/switchxkblayout -- keychron-keychron-k2 0` | `device not found` |
| `/dispatch switchxkblayout keychron-keychron-k2 0` | `Invalid dispatcher` |
| `switchxkblayout keychron-keychron-k2 0` | `ok` |

Follow-ups:

- Class defaults rely on `activewindow` arriving before its `activewindowv2`. If that ever breaks, track classes from `openwindow` or query `j/clients` for unknown addresses.
- Socket paths are found once at startup. If reconnects keep failing after a Hyprland restart, rediscover them before retrying.
