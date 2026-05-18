# Benchmark Notes

These measurements were taken on 2026-05-18 in a live Hyprland session with `kb_layout = us,th` and ten devices reported under `keyboards` by `hyprctl devices -j`.

The benchmark compared spawning `hyprctl` with sending the same commands directly to Hyprland's `.socket.sock`.

| Operation | Mean | p50 | p95 |
| --- | ---: | ---: | ---: |
| `hyprctl getoption input:kb_layout -j` | 1194-1261 us | 1097-1155 us | 1672-1721 us |
| direct `j/getoption input:kb_layout` | 4-5 us | 4-5 us | 6 us |
| `hyprctl switchxkblayout` for one keyboard | 1153-1198 us | 1111-1136 us | 1292-1503 us |
| direct `switchxkblayout` for one keyboard | 5-9 us | 5-9 us | 6-10 us |
| `hyprctl switchxkblayout` for ten keyboard entries | 11429 us | 11425 us | 12335 us |
| direct `switchxkblayout` for ten keyboard entries | 57 us | 58 us | 64 us |

Microbenchmarks also checked event parsing and state choices:

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

- Use direct Hyprland IPC. This is the only measured optimization with a large latency impact.
- Raw Hyprland command IPC for `switchxkblayout` does not use the `--` separator that may appear in CLI-style usage. The verified direct IPC request format is `switchxkblayout <keyboard> <index>`.
- Use a single plain runtime state struct instead of global mutexes. The performance win is small, but the design is simpler and avoids lock lifetime mistakes.
- Use low-allocation event parsing and numeric window addresses. This is simple and removes avoidable hot-path allocations.
- Precompute default layouts by class in a `HashMap`. Small configs do not need it for speed, but it keeps the runtime path simple and scales better for large configs.
- Do not add more micro-optimizations unless a new benchmark shows a real bottleneck.

Direct IPC format verification:

| Request | Result |
| --- | --- |
| `switchxkblayout -- keychron-keychron-k2 0` | `device not found` |
| `/switchxkblayout -- keychron-keychron-k2 0` | `device not found` |
| `/dispatch switchxkblayout keychron-keychron-k2 0` | `Invalid dispatcher` |
| `switchxkblayout keychron-keychron-k2 0` | `ok` |

Known follow-up items:

- The current window default-layout path relies on Hyprland sending `activewindow` before the matching `activewindowv2`. Keep this simple unless real testing shows ordering problems. TODO: either use `openwindow` to maintain an address-to-class map, or query `j/clients` when a new address is seen without a reliable class.
- Socket paths are discovered once at startup. If reconnect repeatedly fails after a Hyprland restart or instance path change, rediscover `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE` before the next reconnect attempt.
