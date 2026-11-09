//! `nyx-desktop-widgets-backend`: the desktop widgets without GTK, for their Quickshell
//! interface (`qml/DesktopWidgets` in this repository), on NyxOS.
//!
//! Plugins, the layout file, placement, sizes and visibility are worked out here as the
//! GTK host did (`host.rs`, `screens.rs`); the interface draws one bottom-layer surface
//! per monitor and talks to this over `$XDG_RUNTIME_DIR/nyx-desktop-widgets/ui.sock`
//! (nyx-ui-link):
//!
//! - topic `widgets`: `{editing, screens: [{connector, area, tiles: […]}], gallery,
//!   popover}`; each tile's `item.view` is its plugin's tree with paths resolved, and
//!   `item.rev` goes up only when the plugin sends it again;
//! - event `Patch` (`{plugin, item, monitor, nodes}`): a plugin's patch, applied to the
//!   view on screen;
//! - operations: `Click`, `ViewEvent`, `MenuAction`, `PopoverClosed`, `Edit`, `Move`,
//!   `ResizeFree`, `CycleSize`, `ToggleBackground`, `ToggleWorkspace`, `Remove`,
//!   `GalleryOpen`, `GalleryClose`, `Add`.
//!
//! It owns `io.github.design_nexus.nyx.DesktopWidgets` and serves `DesktopWidgets1`, as
//! the GTK host did. `NYX_DW_ONLY=<output>` with `NYX_DESKTOP_WIDGETS_UI_SOCKET` makes a
//! test copy for one output (bus name `…DesktopWidgets.Test`).

mod dbus;
mod host;
mod screens;

use gio::prelude::*;
use serde_json::{Value, json};

pub const APP_ID: &str = "io.github.design_nexus.nyx.DesktopWidgets";

fn handle(s: &std::rc::Rc<host::Shared>, req: nyx_ui_link::Request) {
    let b = req.body.clone();
    let str_of = |k: &str| b.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let int_of = |k: &str| b.get(k).and_then(Value::as_i64).unwrap_or(0) as i32;
    let (connector, id) = (str_of("monitor"), str_of("id"));
    let done = |ok: bool| if ok { json!({}) } else { json!({"changed": false}) };
    let reply = match req.op.as_str() {
        "Click" => screens::click(s, &connector, &id, b.get("button").and_then(Value::as_u64).unwrap_or(1) as u32),
        "ViewEvent" => {
            screens::view_event(s, &connector, &id, &str_of("kind"), b.get("control").and_then(Value::as_str).map(String::from), b.get("value").cloned());
            json!({})
        }
        "MenuAction" => {
            screens::view_event(s, &connector, &id, "action", Some(str_of("action")), None);
            json!({})
        }
        "PopoverClosed" => {
            screens::close_popover(s);
            json!({})
        }
        "Edit" => {
            screens::set_editing(s, b.get("on").and_then(Value::as_bool).unwrap_or(true));
            json!({})
        }
        "Move" => done(screens::move_to(s, &connector, &id, (int_of("x"), int_of("y")))),
        "ResizeFree" => done(screens::resize_free(s, &connector, &id, int_of("w"), int_of("h"))),
        "CycleSize" => done(screens::cycle_size(s, &connector, &id)),
        "ToggleBackground" => done(screens::toggle_background(s, &connector, &id)),
        "ToggleWorkspace" => done(screens::toggle_workspace(s, &connector, &id)),
        "Remove" => done(screens::remove(s, &connector, &id)),
        "GalleryOpen" => {
            screens::open_gallery(s, &connector);
            json!({})
        }
        "GalleryClose" => {
            screens::close_gallery(s);
            json!({})
        }
        "Add" => {
            screens::add_widget(s, &connector, &str_of("key"), &str_of("size"));
            json!({})
        }
        _ => {
            let op = req.op.clone();
            return req.err("unknown_op", op);
        }
    };
    req.ok(reply);
}

fn main() -> std::process::ExitCode {
    let mut only = std::env::var("NYX_DW_ONLY").ok().filter(|s| !s.is_empty());
    let mut edit = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--only" => only = args.next(),
            "--edit" => edit = true,
            "-h" | "--help" => {
                println!(
                    "Usage: nyx-desktop-widgets-backend [--edit] [--only <output>]\n\n\
                     The desktop widgets' services for their Quickshell interface (NyxOS).\n\
                     --edit   start in edit mode (or, while it runs: turn edit mode on)\n\
                     --only   widgets on this output only (e.g. HEADLESS-1), as a test copy"
                );
                return std::process::ExitCode::SUCCESS;
            }
            other => {
                eprintln!("nyx-desktop-widgets-backend: unknown option {other}");
                return std::process::ExitCode::from(2);
            }
        }
    }
    if edit && only.is_none() && dbus::call_running("Edit", Some(&(true,).to_variant())) {
        return std::process::ExitCode::SUCCESS;
    }
    let ctx = glib::MainContext::default();
    let _guard = ctx.acquire().expect("main context");
    let (req_tx, req_rx) = async_channel::unbounded::<nyx_ui_link::Request>();
    let link = match nyx_ui_link::serve(nyx_ui_link::Options::new("nyx-desktop-widgets"), move |r| {
        let _ = req_tx.send_blocking(r);
    }) {
        Ok(l) => l,
        Err(nyx_ui_link::Error::AlreadyRunning(p)) => {
            eprintln!("nyx-desktop-widgets-backend: already running at {}", p.display());
            return std::process::ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("nyx-desktop-widgets-backend: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let s = host::start(link, only, edit);

    match gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
        Ok(conn) => {
            if let Err(e) = dbus::serve(&conn, &s) {
                eprintln!("nyx-desktop-widgets-backend: couldn't serve DesktopWidgets1: {e}");
            }
            let name = if s.test() { format!("{APP_ID}.Test") } else { APP_ID.to_string() };
            gio::bus_own_name_on_connection(
                &conn,
                &name,
                gio::BusNameOwnerFlags::REPLACE | gio::BusNameOwnerFlags::ALLOW_REPLACEMENT,
                |_, _| {},
                |_, name| eprintln!("nyx-desktop-widgets-backend: {name} is taken (is the GTK host running?)"),
            );
            std::mem::forget(conn);
        }
        Err(e) => eprintln!("nyx-desktop-widgets-backend: no session bus: {e}"),
    }

    let s2 = s.clone();
    ctx.spawn_local(async move {
        while let Ok(req) = req_rx.recv().await {
            handle(&s2, req);
        }
    });
    glib::MainLoop::new(Some(&ctx), false).run();
    std::process::ExitCode::SUCCESS
}
