//! `io.github.design_nexus.nyx.DesktopWidgets1` on the session bus: edit mode, what
//! runs and what it uses, and moving widgets (which tests use instead of input).

use crate::host::{self, Shared};
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;

pub const PATH: &str = "/io/github/design_nexus/nyx/DesktopWidgets";
pub const INTERFACE: &str = "io.github.design_nexus.nyx.DesktopWidgets1";

const XML: &str = r#"<node>
  <interface name="io.github.design_nexus.nyx.DesktopWidgets1">
    <method name="Edit"><arg type="b" name="on" direction="in"/></method>
    <method name="ListWidgets"><arg type="s" name="json" direction="out"/></method>
    <method name="Move">
      <arg type="s" name="part" direction="in"/><arg type="s" name="monitor" direction="in"/>
      <arg type="s" name="anchor" direction="in"/><arg type="i" name="x" direction="in"/><arg type="i" name="y" direction="in"/>
      <arg type="b" name="found" direction="out"/>
    </method>
    <method name="Resize">
      <arg type="s" name="part" direction="in"/><arg type="s" name="monitor" direction="in"/><arg type="s" name="size" direction="in"/>
      <arg type="b" name="found" direction="out"/>
    </method>
    <method name="Reload"/>
    <method name="RestartPlugin"><arg type="s" name="id" direction="in"/><arg type="b" name="found" direction="out"/></method>
    <method name="TestGallery"/>
    <method name="TestClick">
      <arg type="s" name="part" direction="in"/><arg type="s" name="monitor" direction="in"/><arg type="u" name="button" direction="in"/>
      <arg type="b" name="found" direction="out"/>
    </method>
  </interface>
</node>"#;

pub struct Registration {
    _id: gio::RegistrationId,
}

/// Call a method on the running service; false when it isn't running.
pub fn call_running(method: &str, params: Option<&glib::Variant>) -> bool {
    let Ok(conn) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else { return false };
    conn.call_sync(
        Some(crate::APP_ID),
        PATH,
        INTERFACE,
        method,
        params,
        None,
        gio::DBusCallFlags::NO_AUTO_START,
        2000,
        gio::Cancellable::NONE,
    )
    .is_ok()
}

pub fn serve(conn: &gio::DBusConnection, s: &Rc<Shared>) -> Result<Registration, glib::Error> {
    let info = gio::DBusNodeInfo::for_xml(XML)?.lookup_interface(INTERFACE).expect("DesktopWidgets1");
    let ws = Rc::downgrade(s);
    let id = conn
        .register_object(PATH, &info)
        .method_call(move |_, _, _, _, method, params, invocation| {
            let Some(s) = ws.upgrade() else {
                invocation.return_dbus_error("org.freedesktop.DBus.Error.Failed", "the widgets are closing");
                return;
            };
            match method {
                "Edit" => {
                    let (on,) = params.get::<(bool,)>().unwrap_or((true,));
                    host::set_editing(on);
                    invocation.return_value(None);
                }
                "ListWidgets" => {
                    let json = host::list(&s).to_string();
                    invocation.return_value(Some(&(json,).to_variant()));
                }
                "Move" => {
                    let (part, monitor, anchor, x, y) = params.get::<(String, String, String, i32, i32)>().unwrap_or_default();
                    let found = crate::screens::move_widget(&s, &part, &monitor, &anchor, [x, y]);
                    invocation.return_value(Some(&(found,).to_variant()));
                }
                "Resize" => {
                    let (part, monitor, size) = params.get::<(String, String, String)>().unwrap_or_default();
                    let found = crate::screens::resize_widget(&s, &part, &monitor, &size);
                    invocation.return_value(Some(&(found,).to_variant()));
                }
                "Reload" => {
                    host::reload(&s);
                    invocation.return_value(None);
                }
                "RestartPlugin" => {
                    let id = params.get::<(String,)>().map(|(i,)| i).unwrap_or_default();
                    let found = host::restart(&s, &id);
                    invocation.return_value(Some(&(found,).to_variant()));
                }
                "TestGallery" | "TestClick" if std::env::var_os("NYX_DW_DEBUG").is_none() => {
                    invocation.return_dbus_error("org.freedesktop.DBus.Error.AccessDenied", "test methods need NYX_DW_DEBUG")
                }
                "TestGallery" => {
                    crate::screens::test_gallery(&s);
                    invocation.return_value(None);
                }
                "TestClick" => {
                    let (part, monitor, button) = params.get::<(String, String, u32)>().unwrap_or_default();
                    let found = crate::screens::test_click(&s, &part, &monitor, button);
                    invocation.return_value(Some(&(found,).to_variant()));
                }
                _ => invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method),
            }
        })
        .build()?;
    Ok(Registration { _id: id })
}
