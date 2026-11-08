# Development notes

A CPU- and memory-first Apple Silicon monitor with a Rust TUI and a native Swift
menu-bar app, sharing native CPU, memory, swap, and process measurements. No sudo
is required.

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
  The shared sampler also reads numeric pressure as 100 minus
  `kern.memorystatus_level`, the kernel's available-memory percentage. This is
  independent of both RAM usage and the OS pressure state, not an estimate from
  the app/wired/compressed categories. A higher percentage means less headroom.
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
collection. See [third-party notices](../THIRD_PARTY_NOTICES.md) for macmon attribution.

## Native app

The native app is a SwiftPM package under `macos/`, requiring macOS 14+, Apple
Silicon, and Swift 6.2+. It uses AppKit for the application/status-item/popover
lifecycle, SwiftUI for the detail views, and Apple's Swift Charts for CPU
history. No Xcode project, third-party Swift dependencies, web view, server, or
measurement subprocess is involved. The TUI's entry point and commands remain
unchanged.

```sh
./scripts/run-app             # build and open, live readings
./scripts/run-app --demo      # explicit synthetic E+P+S fixture
./scripts/build-app release   # build only
```

The build script compiles the Rust static library, builds Swift, and produces
an ad-hoc-signed `macos/.build/Mactop.app`. It does not install the app, add a
login item, or enable a privileged helper. Left-click CPU or MEM for details;
right-click either to choose 0.2, 0.5, 1, 2, or 4 Hz, or Quit. The selection
controls the shared sampler, applies to both items, and lasts for the current
run (default: 1 Hz). MEM reports in-use RAM, not a pressure percentage.

The 312 pt popovers omit redundant titles and lead with CPU activity or in-use
memory percentage. CPU places its fixed 60-second trend beside the reading,
with 10 pt core cells below: green efficiency, blue performance, pink Super.
Cells wrap instead of stretching; interval/max clocks and individual core values
are available on hover. Memory places numeric pressure beside usage, both in
neutral text, above a compact capacity strip and three-column app/wired/compressed
key. Swap usage sits in a footer below the processes. Capacity, cache/free,
pressure definitions, swap allocation and paging rates are available on hover;
elevated or critical pressure also shows a warning, without a normal-state badge.
Both show four processes, with application icons where available. Quit stays in
the right-click menu.

Ownership and data flow:

- `src/sampling.rs` is shared by the TUI and the C bridge. Its single joined
  worker owns the native handles, primes counters, and collects at 1 Hz by
  default. Its latest-only slot bounds delivery.
- `src/ffi.rs` and `macos/Sources/CMactop/include/mactop.h` define the internal
  typed C interface. The handle retains its snapshot; exported arrays and UTF-8
  strings borrow that storage until the next poll or destruction. Calls on one
  handle must be serialized. Frequencies use NaN for unavailable readings; no
  strings, processes, core tiers, or clocks are silently omitted by the bridge.
- `MetricsCore` copies those views into owned, immutable `Sendable` Swift
  values before polling again. It preserves monotonic timestamps, sequence,
  reset/delay flags, and unavailable frequencies. This is an internal interface,
  not a versioned SDK or a plugin API.
- `MonitorStore` owns one background Swift task for the sampler's complete
  lifetime. Only owned readings/errors cross to MainActor. Both status items
  share that store. Startup failures retry after five seconds; measurement
  errors retain the last good reading and retry on subsequent polls. Rate
  changes interrupt the current wait and reschedule collection without resetting
  counters. Rescheduling waits for the new collection deadline. A poll just ahead
  of collection retries once after 50 ms, then returns to the selected interval
  if no sample is ready. Delivery waits use monotonic deadlines so view
  updates do not add to the selected interval. No delivery for three intervals
  (at least three seconds) marks the reading stale. Unexpected worker termination
  is surfaced, but requires restarting the app rather than silently rebuilding it.
- Each reading, its history, and its error/staleness state publish together.
  Status items observe only their rounded percentage and staleness, updating
  their images and accessibility text synchronously when those values change.
- CPU history uses the sampler's monotonic timestamps, retaining the last
  minute and one left-edge predecessor. Errors, missed sequences and delayed
  samples break the line; restarts clear it. Rate changes preserve the time
  scale. Startup history is blank until measured, and stale history freezes.
- Quit cancels and awaits the task; deferred bridge destruction stops/joins
  Rust's worker before AppKit receives permission to terminate. Programmatic
  quit is scheduled outside the current Swift executor job so AppKit's nested
  termination loop cannot block that shutdown.

The popover host follows Veil's fitting-size update on every layout, including
loading → data → error → recovery while the same popover stays open. Debug builds
can capture the actual popover with ScreenCaptureKit (macOS 14.4+):

```sh
./scripts/build-app
macos/.build/Mactop.app/Contents/MacOS/Mactop \
  --demo --capture /tmp/mactop-cpu.png --panel cpu --appearance light --check-transitions
```

Use `--panel memory` or `--appearance dark` for the other views. Omit `--demo`
for native readings. Transition checking injects an explicitly labelled UI
error, not a native collector fault; it saves loading, error and recovered
captures alongside the requested PNG and checks that the open window grows and
shrinks. Add `--capture-menu` to capture the actual frequency menu, exercise its
slow/fast actions against collection, and verify both items share the selection.
Add `--capture-history` to wait for a full minute of measured CPU history.
Capture tooling is excluded from release builds. The normal app needs neither
screen-recording nor Accessibility permissions.

## Check

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
./scripts/build-app
swift test --package-path macos
xcrun swift-format lint --strict --recursive macos/Sources macos/Tests
```

The command-line-only Swift 6.4 beta toolchain on the development host currently
requires its installed Testing macro plugin to be loaded explicitly. With that
toolchain, replace the `swift test` command with:

```sh
toolchain="$(dirname "$(dirname "$(xcrun --find swift)")")"
swift test --package-path macos \
  -Xswiftc -load-plugin-library \
  -Xswiftc "$toolchain/lib/swift/host/plugins/testing/libTestingMacros.dylib"
```

The native app has been run on the same M4 Pro with macOS 27; macOS 14 is the
deployment target, not an additional hardware/OS validation claim.
