//! Nyx Desktop Widgets: widgets on the desktop, behind windows, on every monitor. The
//! host only hosts: every widget is a `desktop` part of a plugin (see nyx-bar's
//! `docs/plugins.md`). `nyx plugin` turns parts on; edit mode arranges them.
//!
//! `--only <connector>` shows widgets on one output only, as a separate test copy that
//! never takes the keyboard. `--edit` starts in edit mode.

mod dbus;
mod host;
mod screens;
mod theme;

use gtk::prelude::*;

pub const APP_ID: &str = "io.github.design_nexus.nyx.DesktopWidgets";

fn wants_gpu() -> bool {
    nyx_desktop_widgets_core::Layout::load(&host::layout_path()).ok().and_then(|l| l.renderer).is_some_and(|r| r == "gpu")
}

fn main() -> std::process::ExitCode {
    let mut only = std::env::var("NYX_DW_ONLY").ok().filter(|s| !s.is_empty());
    let mut edit = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--only" => only = args.next(),
            "--edit" => edit = true,
            "--version" => {
                println!("nyx-desktop-widgets {}", env!("CARGO_PKG_VERSION"));
                return std::process::ExitCode::SUCCESS;
            }
            "-h" | "--help" => {
                println!(
                    "Usage: nyx-desktop-widgets [--edit] [--only <output>]\n\n\
                     Runs the desktop widgets (normally as the nyx-desktop-widgets user service).\n\
                     --edit   arrange widgets (or, while it runs: the same command again)\n\
                     --only   show widgets on this output only (e.g. HEADLESS-1), as a test copy\n\n\
                     Widgets: `nyx plugin add`, `nyx plugin configure`.\n\
                     Layout: ~/.config/nyx/desktop-widgets.toml. Style: ~/.config/nyx/desktop-widgets.css."
                );
                return std::process::ExitCode::SUCCESS;
            }
            other => {
                eprintln!("nyx-desktop-widgets: unknown option {other}");
                return std::process::ExitCode::from(2);
            }
        }
    }
    // Widgets redraw little: cairo draws them for far less memory than the GPU
    // renderers. `renderer = "gpu"` in the layout file leaves it to GTK.
    if !wants_gpu() {
        unsafe { std::env::set_var("GSK_RENDERER", "cairo") };
    }
    let (id, flags) = match &only {
        Some(_) => (format!("{APP_ID}.Test"), gtk::gio::ApplicationFlags::NON_UNIQUE),
        None => (APP_ID.to_string(), Default::default()),
    };
    let app = gtk::Application::builder().application_id(id).flags(flags).build();
    app.connect_startup(|_| theme::install());
    if edit && only.is_none() && dbus::call_running("Edit", Some(&(true,).to_variant())) {
        return std::process::ExitCode::SUCCESS;
    }
    let started = std::cell::Cell::new(false);
    app.connect_activate(move |app| {
        if !started.replace(true) {
            host::start(app, only.clone(), edit);
        }
    });
    let code = app.run_with_args::<&str>(&[]);
    std::process::ExitCode::from(code.get())
}
