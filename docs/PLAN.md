# Plan: Nyx Desktop Widgets

Status: version 1 built (2026-11-03), on NyxOS only (never installed on Ken's laptop).
The plan moved here from `nyx/docs/desktop/`. It builds on:

- the shared plugin system from the Nyx Bar plan (`nyx-bar/docs/PLAN.md`);
- the crate move in the Control Center plan (`nyx-control-center/docs/PLUGIN_PLAN.md`),
  not done yet: this repo pins nyx-bar's plugin crates until then;
- ARCHITECTURE.md §4.6.

### Built, and where it differs

- The host, layout file, edit mode (grid, drag, size, background, remove, **Add
  widget**), `DesktopWidgets1`, click-through outside interactive widgets, pausing
  under windows and fullscreen, and four first-party widgets: `nyx.clock/large`,
  `nyx.weather/card`, `nyx.stats/graphs`, `nyx.media/player`.
- Instead of `widget_open` and `resize`, `hello` carries a `widgets` list and a
  `widgets` message repeats it whenever one is added, removed, moved or resized. One
  idempotent message is simpler for scripts, and interval plugins get it in `hello`.
- The repo is `nyxos-project/nyx-desktop-widgets`, and NyxOS packages it; it isn't in
  `suite.toml` or Omarchy's `hypr/nyx.lua`.
- Headless outputs are skipped unless `--only` names one.
- A monitor whose widgets are all hidden has no surface at all (an empty one is
  configured by Hyprland on every frame).

### Added after version 1

- Free sizes: a part with `resizable = true` (and `min`, `max`) gets a grip in edit
  mode; `size = "free"` with `width` and `height` in the layout. The stats graphs are
  resizable.
- Popovers on click and menus on right click, on interactive widgets, as in the bar.
- Pausing while the session is locked, from logind's `LockedHint` (untested here:
  it needs the locker to set the hint).
- The suite's text scale: named sizes grow with it, and plugins' `font_size` follows
  it in every host.
- **Add widget** draws live previews: while the list is open, the plugins of the parts
  in it run with a widget on monitor `preview`.
- `nyx.upnext/card`, in this repo (`plugins/upnext`), from Nyx Calendar's agenda.
- Hourly weather on the large weather card, from `nyx-weather --json`.

- Phase 2, per-workspace layouts: `[[all.workspace."<n>".widget]]` and
  `[[monitor."<id>".workspace."<n>".widget]]` (a workspace's id or name). Every
  workspace's widgets are built once; a switch maps and unmaps them, and plugins hear
  `visible` per item. A workspace's widget replaces the same part's shared one there.
  Edit mode's pin moves a widget between every workspace and the active one. A part on
  several workspaces of one monitor is one view, so its `widgets` entry gives the size
  of the one showing, again on each switch.

### Measured

On this laptop, release build, a 1600×1000 headless output, Hyprland busy with
terminal titles changing several times a second:

| | Private | PSS | CPU | Wake-ups |
| --- | --- | --- | --- | --- |
| No widgets | 12.0 MiB | 19.0 MiB | 0.20% | 17.5/s |
| Five widgets | 17.8 MiB | 29.3 MiB | 0.27% | 19.8/s |
| Five widgets, all covered | 18.4 MiB | 27.1 MiB | 0.20% | 18.1/s |

Nearly every wake-up is the Hyprland watcher (the same one the bar has). Each widget
plugin is its own process besides: stats about 0.5 MiB, media about 1.4 MiB in their
cgroups; the clock and weather run only on their minute and ten minutes.

## Goal

The goal is a host that shows widgets on the desktop, behind windows, on every monitor.
Every widget is a plugin part of the `desktop` kind, so new widgets never need a rebuild.
Users choose parts with `nyx plugin add` or `nyx plugin configure`, and arrange them in an
edit mode. Version 1 is per monitor, as the NyxOS brief asks. Per-workspace layouts come
second, using the cached-tree swap described in §4.6.

## Host

- **Process:** `nyx-desktop-widgets`, app ID `io.github.design_nexus.nyx.DesktopWidgets`,
  started by a systemd user unit in `graphical-session.target`. Hidden test copies use
  `--only <connector>` (or `NYX_DW_ONLY`), which makes the app non-unique, as the dock
  does.
- **Surfaces:** one layer-shell surface per monitor.
  - Namespace `nyx-desktop-widgets`, `Layer::Bottom`, anchored on all four edges.
  - Exclusive zone 0, so the surface respects the bar's zone.
  - Keyboard mode `None`.
  - Omarchy's `omarchy.background` stays on the background layer underneath.
  - The surfaces rebuild on hotplug like `rebuild_docks` in nyx-dock.
- **Click-through by default:** the input region is empty except for widgets whose part
  sets `interactive = true`, such as media controls. In edit mode the whole surface takes
  input, and keyboard mode becomes `OnDemand`.
- **Pausing:** a widget is `visible = false` when windows on the monitor's active
  workspace cover its rectangle (from the `nyx_desktop` window geometry), when a
  fullscreen window is up, or when the session is locked. Its plugin gets `visible` and
  should pause, and the host pauses its CSS animations and frames. A monitor whose
  widgets are all covered costs nothing.
- **Theme:** `nyx_theme::install(Style { window_class: "nyx-desktop-widgets", .. })`.
  - Widgets sit on a solid `@theme_card` with `radius_card` and no glass, or on no
    background when the user picks `background = "none"`.
  - Sizes scale with the suite's text scale.

## Manifest and protocol for the `desktop` kind

```toml
[desktop]
mode = "stream"
[[desktop.items]]
id = "card"
name = "Forecast"
sizes = ["compact", "medium", "large"]   # 160×160, 336×160, 336×336 logical px
resizable = false                        # true: free size between min and max
min = [160, 120]
max = [672, 504]
interactive = false
default = false
```

New messages go in a `desktop` module of `nyx-plugin-protocol`:

- Host to plugin:
  - `widget_open`: item, size in px, monitor, scale;
  - `resize`;
  - `visible`;
  - `event`;
  - `desktop`, only with that grant.
- Plugin to host:
  - `widget`: item, `view`;
  - `patch`, `popover` and `menu`.

All four drawing modes are allowed. These limits for third-party plugins apply in
addition to the shared ones:

- a frame can be as large as the widget, at no more than 30 fps;
- frames stop while the widget isn't visible;
- a monitor can hold at most 12 widgets from one plugin.

## Layout

The layout file is `~/.config/nyx/desktop-widgets.toml`. The host watches it, and
`nyx plugin` edits it with `toml_edit`.

```toml
version = 1

[[all.widget]]                         # every monitor
part = "nyx.clock/large"
anchor = "top-left"                    # 9 anchors; offsets in logical px
offset = [48, 48]
size = "large"

[[monitor."LG Electronics|27GN950|012345".widget]]   # stable ID; falls back to connector
part = "nyx.weather/card"
anchor = "top-right"
offset = [48, 48]
size = "medium"
background = "card"
options = { units = "imperial" }
```

- Anchors and offsets keep widgets in place when the resolution or scale changes.
  Offsets snap to an 8 px grid.
- A monitor that's unplugged keeps its layout until it returns (the risk table in
  ARCHITECTURE.md).
- Phase 2 adds `[[monitor."<id>".workspace."<n>".widget]]` and
  `[[all.workspace."<n>".widget]]`.
  - On a workspace switch, the host swaps which cached widget trees are mapped and never
    rebuilds them.
  - Widgets on inactive workspaces are unmapped, and their plugins get `visible = false`.
  - This is the per-workspace approach from §4.6; it needs no layer-shell support per
    workspace.

## Edit mode

- **Entering it:** `nyx-desktop-widgets --edit`, or the D-Bus method below. It doesn't
  bind a key on Omarchy without Ken's say-so; a suggested bind is documented for
  `hypr/nyx.lua`.
- **What changes:**
  - The surface takes input and shows a faint 8 px grid.
  - Each widget gets a drag handle, a size cycle, a background toggle and a remove button.
- **Moving and resizing:** dragging snaps to the grid. The anchor is recomputed from the
  nearest third of the screen, so a widget dragged to the right edge stays right-anchored.
- **Adding widgets:** the **Add widget** gallery lists every enabled `desktop` part, with
  a live preview at its default size.
  - Installed parts that aren't enabled appear under "Turn on in a terminal:
    `nyx plugin configure <id>`".
  - The host never approves third-party permissions itself.
- **Missing plugins:** a placed widget whose plugin is missing shows as a placeholder in
  edit mode and can be removed.
- **Leaving:** Esc or **Done**.

**D-Bus:** `io.github.design_nexus.nyx.DesktopWidgets1` has these methods:

- `Edit(b)`;
- `ListWidgets`, which returns placement, state, grants and cgroup memory and CPU;
- `Move(part, monitor, anchor, x, y)`;
- `Resize(part, monitor, size)`;
- `Reload`;
- `RestartPlugin(id)`.

Tests drive edit mode through this interface, never through synthetic input.

## First-party widgets

These are `desktop` parts added to the first-party plugins in nyx-bar, so each plugin
stays one folder serving several kinds:

- `nyx.clock/large`: the time and date in large type; compact shows just the time.
- `nyx.weather/card`: current conditions plus the next hours, through
  `nyx-weather --waybar`.
- `nyx.stats/graphs`: CPU, memory and temperature graphs for the last 10 minutes, drawn
  as vectors.
- `nyx.media/player`: album art, title and controls. It's `interactive = true`, so only
  its rectangle takes clicks.
- `nyx.upnext/card`: new in this repo. The next calendar events from `nyx-calendard`,
  and only when Calendar is installed.

## Shared code

- **Plugins:** the host uses `nyx-plugin`, `nyx-plugin-host` and `nyx-plugin-protocol`
  from nyx-core, and `nyx-plugin-view` from nyx-design (after the Control Center plan's
  move).
- **System data:** ARCHITECTURE.md §10 says `nyx-widgets` and `nyx-providers` move when
  desktop widgets becomes their second user. Since widgets are now plugins, the move
  becomes this:
  - `nyx-providers` (system data) moves to nyx-core;
  - `nyx-plugin-sdk`'s `network`, `bluetooth`, `audio`, `power` and `mpris` services use
    it, so the Control Center and every plugin share one implementation;
  - `nyx-widgets` (`Live`, sizes, page packing) stays in the Control Center, because
    nothing else needs it.

## Security and resource limits

The same model as the bar and Control Center: bubblewrap for third-party plugins,
approved permissions, the shared D-Bus proxy, `host_commands`, and transient units
named `nyx-plugin-desktop-<id>.service` with the same default limits.
Widgets that open windows need the `windows` grant, as in the bar.

## Measurements

One more GTK process: the architecture doc measured a minimal layer-shell process at
about 33 MiB private memory. Add one process for each widget plugin that's running.
Record private memory, PSS, idle wakeups and CPU on this laptop with
`docs/desktop/bench/`, with:

- no widgets;
- the five first-party widgets;
- the same five fully covered by windows, which should be near zero CPU.

## Testing

All on a headless output (the screenshot rule), with scratch XDG directories and
`nyx-desktop-widgets --only HEADLESS-<n>`:

- `cargo test`:
  - layout parsing;
  - anchor and offset maths across resolutions and scales;
  - snapping;
  - stable monitor IDs with the connector fallback;
  - orphaned layouts are kept;
  - `toml_edit` placement keeps comments.
- Screenshots: the five first-party widgets at each size, with card and with no
  background, in a light and a dark theme.
- Visibility: open a test window covering a widget, and check that `visible = false`
  reaches the plugin and frames stop. Then check that fullscreen and a locked session
  pause everything.
- Input: outside edit mode, clicks pass through everywhere except an `interactive`
  widget. Check the input region by reading back the surface's region in the debug log,
  never by moving the pointer.
- Edit mode through `DesktopWidgets1`: `Edit(true)`, `Move`, `Resize`, then the gallery
  and placeholder screenshots.
- Sandbox and limits: the same denial and flood tests as the bar, with a desktop test
  plugin.
- Install with `./install.sh </dev/null`, and confirm `~/.local/bin/nyx-desktop-widgets`
  has a fresh timestamp.

## Repo and packaging

- Ask Ken before creating `design-nexus/nyx-desktop-widgets` on GitHub. Lay it out like
  nyx-dock, add it to `suite.toml`, and add a layer rule for `nyx-desktop-widgets` to the
  shared `hypr/nyx.lua` rules (ARCHITECTURE.md §5).
- NyxOS: `pkgs/nyx-desktop-widgets.nix` and a `nyx.desktopWidgets.enable` module in
  nyx-packages. Coordinate with the NyxOS session first, as in the bar plan.

## Not in this plan

- Per-workspace wallpapers (§9.2, still deferred).
- Settings pages.
- Widgets on the lock screen.
- Approving permissions from inside the host.
