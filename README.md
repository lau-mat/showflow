# Tauri + Vanilla

This template should help get you started developing with Tauri in vanilla HTML, CSS and Javascript.

## Run sheet times

The editor, printed run sheets, and client cue displays show each item's absolute
start time as `HH:MM:SS` in local time. Items without an explicit start time begin
at the previous item's start time plus its duration, or at the show start time
for the first item. An explicit item start time resets the schedule for following
items.

## Session timers

Session `.timer` elements use `data-source="system-clock"` to display local
wall-clock time immediately. Each timer has its own 100 ms update loop; the
readout changes only when its formatted value changes, while the remaining-time
bar transitions smoothly between updates. `data-label` sets the
label, and `data-format` selects and orders the zero-padded `HH`, `MM`, and `SS`
tokens. For example, `HH-MM-SS` (the default) displays `09:05:03`, `MM-SS`
displays `05:03`, and `HH:MM` displays `09:05`. Hyphens are component separators
rendered as colons. Label, source, and format attributes are reread each tick.

The shared timer renderer in `src/js/timers.js` obtains time components from
`timerSources`; future sources can supply the same `HH`, `MM`, and `SS`
components without duplicating formatting or the update loop. `startTimers`
returns a cleanup function to stop updates when a caller removes its timers.

`show-duration` counts up from the first Next Cue press, staying at zero until
then. `current-item` counts down from the item's configured duration whenever
Next Cue, Previous Cue, or a cue jump is received, and goes negative when overdue.
Its remaining-time bar drains from full to empty over the configured duration;
the empty track represents elapsed time. Overdue items have a red-tinted readout
and track. Changing cues resets both the bar and overdue styling. Zero-duration
items have an empty bar; no active item hides the bar.
During the final 15 seconds above zero, the current item turns amber with a
gentle pulsing glow. At overtime, red styling replaces the warning. Reduced-motion
preferences disable the pulse and bar transitions.
Scheduled show/item times do not affect these timers, so rehearsals can start
at any time. The server stores actual start timestamps in the active session
and includes them in cue-change broadcasts and session snapshots, keeping
timers consistent across page reloads. A new session resets both timestamps.
Both sources update from the latest show/session data each tick; unavailable data
or no active item displays dashes. Duration formats without `HH` use total minutes,
so `MM-SS` displays `65:00` rather than wrapping after an hour. The session passes
its current data through the optional state callback to `startTimers`.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
