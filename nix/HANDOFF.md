# Packaging Nyx Desktop Widgets for NyxOS

For the session working on `nyx-packages` and `nyxos`. Nyx Desktop Widgets is pushed
at `nyxos-project/nyx-desktop-widgets`; its build was tested on Arch, not yet under
Nix. It needs nyx-bar at 34b7c0c or later, whose first-party plugins carry the
desktop parts (clock, weather, stats, media).

## nyx-packages

1. `flake.nix` inputs: `nyx-desktop-widgets = { url = "github:nyxos-project/nyx-desktop-widgets"; flake = false; };`
2. Copy [nyx-desktop-widgets.nix](nyx-desktop-widgets.nix) to
   `pkgs/nyx-desktop-widgets.nix` and add `"nyx-desktop-widgets"` to the `fromSource`
   list in `pkgs/default.nix`.
3. Bump the nyx-bar input to 34b7c0c or later: its plugins folder has the desktop
   scripts (`clock-desktop.sh`, `weather-desktop.sh`), which `nyx-bar.nix` already
   installs with the other `*.sh` files.
4. `modules/nixos/default.nix`: add `desktopWidgets = "Nyx Desktop Widgets";` to
   `apps`. The unit starts with the graphical session through `passthru.autostart`.
5. A check: `nyx.testWorkspace { pname = "nyx-desktop-widgets-tests"; src =
   inputs.nyx-desktop-widgets; buildInputs = [ gtk4-layer-shell librsvg ]; }`.

## nyxos

- `modules/nixos/apps.nix`: `nyx.desktopWidgets.enable = lib.mkDefault true;`.
- A starting layout, if wanted, as `~/.config/nyx/desktop-widgets.toml` from home
  manager (only when the file doesn't exist; the user's edits must survive):

  ```toml
  version = 1

  [[all.widget]]
  part = "nyx.clock/large"
  anchor = "top-left"
  offset = [48, 48]
  size = "large"
  background = "none"
  ```

- The widgets plugins come from nyx-bar's package, so `nyx.bar.enable` (or at least
  the package on `XDG_DATA_DIRS`) is needed for the first-party widgets.

## On the VM, check

- `nyx plugin enable nyx.clock/large --kind desktop` puts the clock top left on every
  monitor; it's under windows and clicks pass through it.
- `nyx-desktop-widgets --edit` shows the grid; widgets drag, resize and come off.
- `gdbus call --session --dest io.github.design_nexus.nyx.DesktopWidgets --object-path
  /io/github/design_nexus/nyx/DesktopWidgets --method
  io.github.design_nexus.nyx.DesktopWidgets1.ListWidgets` lists them.
