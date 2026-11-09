# Nyx Desktop Widgets for nyx-packages (copy to pkgs/nyx-desktop-widgets.nix). The
# widgets themselves are plugins: Up next ships here, the others (clock, weather,
# stats, media) with nyx-bar, all in share/nyx/plugins. NyxOS runs the Quickshell
# interface: nyx-desktop-widgets-backend with the component in
# share/nyx-shell/components, drawn by Nyx Shell's nyx-shell-qs; the GTK host stays
# installed (its unit isn't started).
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
  cargoBuildFlags = [ "-p" "nyx-desktop-widgets" "-p" "nyx-desktop-widgets-backend" "-p" "nyx-desktop-upnext" ];
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
  userUnits = [
    "data/nyx-desktop-widgets.service"
    "data/nyx-desktop-widgets-backend.service"
  ];
  passthru.autostart."nyx-desktop-widgets-backend" = "graphical-session.target";
  postInstall = ''
    mkdir -p $out/share/nyx-shell/components
    cp -r qml/DesktopWidgets $out/share/nyx-shell/components/nyx-desktop-widgets
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
