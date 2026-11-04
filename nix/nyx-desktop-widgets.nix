# Nyx Desktop Widgets for nyx-packages (copy to pkgs/nyx-desktop-widgets.nix). The
# widgets themselves are plugins: the first-party ones (clock, weather, stats, media)
# ship with nyx-bar in share/nyx/plugins.
{
  nyx,
  src,
  gtk4-layer-shell,
  librsvg,
  bubblewrap,
  xdg-dbus-proxy,
  systemd,
  libnotify,
}:
nyx.buildApp {
  pname = "nyx-desktop-widgets";
  inherit src;
  cargoBuildFlags = [ "-p" "nyx-desktop-widgets" ];
  buildInputs = [
    gtk4-layer-shell
    librsvg
  ];
  # Plugins run in systemd scopes and, third-party ones, in bubblewrap.
  runtimeInputs = [
    bubblewrap
    xdg-dbus-proxy
    systemd
    libnotify
  ];
  userUnits = [ "data/nyx-desktop-widgets.service" ];
  passthru.autostart."nyx-desktop-widgets" = "graphical-session.target";
  meta = {
    description = "Plugin widgets on the desktop, behind windows";
    homepage = "https://github.com/nyxos-project/nyx-desktop-widgets";
    mainProgram = "nyx-desktop-widgets";
  };
}
