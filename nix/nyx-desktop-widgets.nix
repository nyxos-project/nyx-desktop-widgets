# Nyx Desktop Widgets for nyx-packages (copy to pkgs/nyx-desktop-widgets.nix). The
# widgets themselves are plugins: Up next ships here, the others (clock, weather,
# stats, media) with nyx-bar, all in share/nyx/plugins.
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
  cargoBuildFlags = [ "-p" "nyx-desktop-widgets" "-p" "nyx-desktop-upnext" ];
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
  postInstall = ''
    dest=$out/share/nyx/plugins/nyx.upnext
    install -Dm644 plugins/upnext/plugin.toml -t "$dest"
    mv "$out/bin/nyx-desktop-upnext" "$dest/"
  '';
  meta = {
    description = "Plugin widgets on the desktop, behind windows";
    homepage = "https://github.com/nyxos-project/nyx-desktop-widgets";
    mainProgram = "nyx-desktop-widgets";
  };
}
