# Spike S1: Hosting and Frame Driving

Code: `spikes/s1-hosting` (run `spikes/target/release/s1-hosting.exe frames|resize|interactive [--source clock|timer|dwmflush]`).

Environment: Windows 11 25H2 (build 26200.9974), AMD Radeon 780M, one 2048×1152 monitor at 59.89 Hz.

## Questions

1. Can a Win32 HWND host a `Windows.UI.Composition` tree with only the `windows` crate?
2. Which frame source should drive animations?
3. When do property changes reach the screen, and do we need explicit commits?
4. How far does content lag a resize?
5. Is the idle cost zero?

## Setup That Works

- `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)`, then `RoInitialize(RO_INIT_SINGLETHREADED)`.
- Window with `WS_EX_NOREDIRECTIONBITMAP`, so DWM doesn't allocate a GDI redirection surface.
- `CreateDispatcherQueueController(DQTYPE_THREAD_CURRENT, DQTAT_COM_STA)` on the UI thread, before creating the `Compositor`.
- `Compositor::new()`, then `ICompositorDesktopInterop::CreateDesktopWindowTarget(hwnd, false)`, then `target.SetRoot(root)`.
- A plain `GetMessageW` loop is enough: the dispatcher queue commits through the thread's message queue.
- Setup of the queue, compositor, target and two visuals takes 3.3–4.7 ms.

Crate notes for `windows` 0.62: the numerics types live in `windows-numerics` and the async types in `windows-future`.
`CreateEventW` and `CreateWaitableTimerExW` need the `Win32_Security` feature.

## Measurements

300 animated frames per source; the sprite offset changes on each frame.
"Commit latency" is the time from a property change to completion of `Compositor::RequestCommitAsync`.

| Frame source | Interval mean (ms) | SD | Min | p99 | Max | Commit latency mean (ms) |
|---|---|---|---|---|---|---|
| `DCompositionWaitForCompositorClock` on a worker thread | 16.699 | 0.239 | 15.67 | 17.29 | 17.57 | 17.9 (SD 0.9) |
| High-resolution waitable timer, 1/60 s, on the UI thread | 17.033 | 0.217 | 16.70 | 17.67 | 18.25 | 11.1 (SD 5.1, 2.1–18.2) |
| `DwmFlush` loop on a worker thread | 16.679 | 0.612 | 13.34 | 18.10 | 18.25 | 16.3 (SD 0.7) |

Display period: 16.697 ms.

- The compositor clock tracks the display period exactly (mean 16.699 ms) with the lowest jitter.
  Commit latency is a steady ~1 frame, so each update lands on the next composition.
- The timer runs at its own rate (17.03 ms here), so it drifts against the display and drops or doubles frames.
  Its commit latency varies with the phase.
- `DwmFlush` works but has three times the jitter of the compositor clock.
- Idle CPU after the animation stops: 0.00 ms over 2 s for every source.
  The worker thread sleeps on an event while nothing animates.

Resize: 31 programmatic `SetWindowPos` calls.
Time from `WM_SIZE` to commit completion: mean 11.1 ms, p50 10.6 ms, max 24.7 ms.
So content can trail the window frame by one to two display frames during a live resize.

## Decisions

- **Frame source**: a dedicated frame thread waits on `DCompositionWaitForCompositorClock` while any animation runs, and posts one coalesced `WM_APP` message per tick to the UI thread (an atomic "pending" flag drops ticks the UI thread hasn't consumed).
  When nothing animates, the thread blocks on an auto-reset event; the UI thread signals it when an animation starts.
  This needs Windows 11, which matches the project's baseline.
- **Commits**: rely on the implicit commit of the dispatcher queue.
  The sync pass runs inside a message handler, so all changes of one pass land in one commit.
  `RequestCommitAsync` is only needed where we must know a frame is on screen (tests, screenshots).
- **Event loop**: `GetMessageW` loop on the UI thread; timers (`update_timers_and_animations`) are driven by the same loop through `SetTimer` / the frame message.
  No `MsgWaitForMultipleObjects` loop is required.
- **DPI**: the window is per-monitor v2 aware.
  The root visual gets its size in DIPs and `Scale = dpi / 96`; whether children snap to physical pixels is decided in S2.

## Open Items for Manual Checks

The only monitor here can't exercise these automatically:

- Run `s1-hosting interactive`, then resize by dragging the border and look for black or stretched edges while the content trails the frame.
  If they are visible, try two mitigations in Phase 1: extend the background visual beyond the client area, or block on `RequestCommitAsync` in `WM_SIZE` during `WM_ENTERSIZEMOVE`…`WM_EXITSIZEMOVE`.
- Move the window between monitors with different scale factors and check that `WM_DPICHANGED` keeps the layout in DIPs.

## Environment Note

In Git Bash, `/usr/bin/link` shadows the MSVC linker and every build script fails with `/usr/bin/link: extra operand`.
Run cargo from PowerShell, or put the MSVC `link.exe` directory first on `PATH`.
