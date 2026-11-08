# mactop

A CPU- and memory-first Rust terminal monitor for Apple Silicon, using native
macOS CPU, memory, swap, and process measurements. No sudo is required.

## Run

```sh
cargo run --release
```

Live data is the default. `cargo run --release -- --demo` selects synthetic
fixtures; it never acts as an automatic fallback. `--sample` prints one native
snapshot after a one-second measurement interval and exits without opening a TUI.

| Key | Action |
| --- | --- |
| `p` | Toggle the dedicated process view |
| `c` / `m` | Sort processes by CPU / resident memory |
| `↑` / `↓`, `k` / `j` | Scroll processes |
| `space` | Freeze / resume the view; native counters keep sampling |
| `f` | Open update frequency: `+` faster, `-` slower, `x` closes |
| `d` | In `--demo`: cycle everyday, CPU load, and memory pressure fixtures |
| `t` | In `--demo`: cycle E+P+S, E+P, and P+S fixtures |
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
border; live/demo/paused and collection-error states also use that border.
Below 90×25, the app asks for a larger terminal. It does not attempt a smaller UI.

Colors follow your terminal: default foreground/background and the ANSI palette,
including in help and process views. There is no forced light or dark theme.

## Measurement boundary

`MetricsSource::sample()` returns a typed `Snapshot`. `MacSource` reads native
APIs; `MockSource` supplies explicit demo fixtures. A single sampling worker owns
the native handles, baselines, and monotonic clocks. A latest-only slot bounds
delivery; the input/render loop never performs measurement calls. No async
runtime or measurement subprocess is needed.

All histories show a fixed 60-second window using monotonic sample timestamps.
The default is one update per second; `f` changes it to 0.2, 0.5, 1, 2, or 4 Hz,
applied immediately for the current run. Changing frequency preserves existing
history and its time scale; it affects only future measurements. Samples expire
by age, with one predecessor retained to clip intervals at the window's edge.
CPU and swap bars cover their measured intervals and preserve bucket peaks;
memory connects timestamped usage readings. Histories grow from the right;
unmeasured time is blank, not fabricated idle or stretched data. Sequence/timestamp
gaps, collection errors, and pause/resume leave breaks rather than erasing earlier
history or connecting across missing measurements. Paused/stale views keep their
last timestamp until a new measurement is displayed. The last good
snapshot remains visible with a STALE indicator after an error; help shows the
error and collection retries. There is no silent mock fallback.

- Tier CPU is active residency, 0–100%; total CPU is weighted by core count.
  Tiers/counts come from sysctl; per-core performance-state residencies come from
  a CPU-only IOReport subscription. Full channel names retain die/cluster/core
  identity. Core numbers in the UI are ordered positions within their tier,
  not Mach logical-CPU IDs.
- GHz is an active-residency-weighted interval average, not an instantaneous
  clock or frequency-scaled utilization. Maximums come from IORegistry SRAM
  DVFS tables. Idle/unavailable readings show `—`, not an invented clock. A
  missing or mismatched frequency table does not suppress valid CPU activity.
- Process CPU uses 100% per logical core, so it can exceed 100%.
  `proc_pidinfo` supplies resident bytes and cumulative Mach CPU time; deltas use
  the native timebase and actual elapsed time. New/reused PIDs start at zero;
  exited or inaccessible processes leave the list. Only processes macOS permits
  the current user to inspect appear; unprivileged launches may omit root-owned
  and other users' processes. Resident sizes can include shared pages and are
  not intended to add up to system app memory.
- In-use memory is app + wired + physically compressed memory, excluding
  reclaimable cache. Available memory is cache + free. Sizes use binary GiB/MiB.
  A single capacity bar shows in-use memory in blue, reclaimable cache shaded
  gray, and free memory as dots. A separate unfilled 0–100% line shows in-use
  history, without stacking cache over it.
- Mach VM statistics use the native page size. App memory is anonymous minus
  purgeable pages; cache is external/file-backed plus purgeable pages. Free
  excludes speculative pages already included in file cache. Compressed is
  physical compressor occupancy. Wired includes installed RAM outside those
  queues (system-reserved/unmanaged memory), so it can exceed `vm_stat`'s wired
  count. These disjoint categories sum to installed physical RAM without calling
  reserved memory free or reclaimable.
- The macOS pressure state stays in the memory border as a normal/elevated/critical
  indicator, separate from the in-use percentage. There is no pressure strip.
- Swap usage and allocation are distinct from swap-in/out rates in bytes/sec.
  The dashboard shows usage and current rates, not allocation. Idle means no
  current paging, even with nonzero swap usage. Both histories share a scale;
  charts appear only if there was activity within the last 60 seconds and remain
  visible after the current rates return to zero.
  Rates use `swapins`/`swapouts` deltas × native page size ÷ elapsed seconds,
  not ordinary page-ins/page-outs or allocated swap. Counter resets produce no
  negative rates or wrapped spikes.
- The E+P+S fixture is illustrative, not a claim about a particular Mac model.

The UI redraws only on a sample, input, or resize; Ratatui diffs its terminal
buffer. Quit stops and joins the worker and releases native handles.

IOReport is a private Apple API, not a stable SDK contract. Live collection has
been exercised on an M4 Pro; other generations require hardware validation.
E/P/S tier routing and newer DVFS layouts are handled conservatively: on
three-tier chips the middle tier's GHz stays unavailable rather than assuming
it shares the highest tier's frequency table. Intel Macs and macOS VMs without
the native Apple Silicon channels are not live targets; `--demo` needs no native
collection. See [third-party notices](THIRD_PARTY_NOTICES.md) for macmon attribution.

## Check

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
