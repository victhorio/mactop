# mactop

A CPU- and memory-first Rust terminal monitor for Apple Silicon. **UI prototype:
all measurements, hardware topologies, process names, and history are synthetic.**
No macOS measurement APIs are connected yet. No root access is needed.

## Run

```sh
cargo run --release
```

| Key | Action |
| --- | --- |
| `p` | Toggle the dedicated process view |
| `c` / `m` | Sort processes by CPU / resident memory |
| `↑` / `↓`, `k` / `j` | Scroll processes |
| `space` | Pause / resume |
| `d` | Cycle everyday, CPU load, and memory pressure fixtures |
| `t` | Cycle E+P+S, E+P, and P+S fixtures |
| `?` or click the border's `?` | Keys and metric definitions; any key or click dismisses help |
| `q` / `Ctrl-C` | Quit |

## Layout

Sizes below are **columns × rows**. Only tiers supplied by the provider appear.

| Size | Tradeoff |
| --- | --- |
| 180×54 | CPU tiers in columns with aligned histories and per-core bars. Memory separates current capacity, usage history, and swap into three columns; fifteen processes. |
| 90×54 | CPU tiers remain in columns with per-core bars. Memory has current capacity on the left and usage history above swap on the right; fifteen processes. |
| 90×25 | CPU tiers remain in columns, using per-core glyphs instead of numeric bars. Memory retains in-use %, available/cache/free, a one-row usage history, and paired swap histories; three processes. |
| 184×25 | Compact CPU columns; memory's three-column layout gives the histories more height than at 90×25; three processes. |

The shorter layouts omit the app/wired/compressed breakdown. At 90×25 the swap
charts also omit their scale label, but keep both current rates and recent activity.
Memory fits its contents in 8–11 rows including its border. CPU grows from 9 to
23 rows; processes use the remaining height through the bottom of the terminal.
Extra height shows more processes instead of padding memory or enlarging charts
indefinitely. Scrolling uses the actual visible process count after each resize.

`p` gives processes the screen rather than squeezing the dashboard further.
There is no app header or command row. Help lives in the process panel's bottom
border; the mock scenario and paused state also use that border, not another row.
Below 90×25, the app asks for a larger terminal. It does not attempt a smaller UI.

Colors follow your terminal: default foreground/background and the ANSI palette,
including in help and process views. There is no forced light or dark theme.

## Measurement boundary

`MetricsSource::sample()` returns a typed `Snapshot`; `MockSource` is its only
implementation today. The app owns bounded 60-sample histories. The renderer
only consumes snapshots and view state. A future macOS collector can replace
the mock without changing the rendering contract. OS calls that block should
live on a sampling worker, not in the input/render loop.

- Tier CPU is active residency, 0–100%; total CPU is weighted by core count.
- Process CPU uses 100% per logical core, so it can exceed 100%.
- In-use memory is app + wired + physically compressed memory, excluding
  reclaimable cache. Available memory is cache + free. Sizes use binary GiB/MiB.
  A single capacity bar shows in-use memory in blue, reclaimable cache shaded
  gray, and free memory as dots. A separate unfilled 0–100% line shows in-use
  history, without stacking cache over it.
- The macOS pressure state stays in the memory border as a normal/elevated/critical
  indicator, separate from the in-use percentage. There is no pressure strip.
- Swap usage and allocation are distinct from swap-in/out rates in bytes/sec.
  The dashboard shows usage and current rates, not allocation. Idle means no
  current paging, even with nonzero swap usage. Both histories share a scale;
  charts appear only if there was activity within the last 60 samples and remain
  visible after the current rates return to zero.
- The E+P+S fixture is illustrative, not a claim about a particular Mac model.

The event loop samples once per second and redraws only on a sample, input, or
resize. Ratatui diffs the terminal buffer; no background task or async runtime is
needed for the mock provider. Rendering is also testable through its in-memory
terminal backend.

## Check

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
