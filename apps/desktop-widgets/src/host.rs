//! The plugins behind the widgets: found in the three plugin folders, run while one of
//! their `desktop` parts is turned on and placed on a connected monitor (and, for
//! third-party plugins, their permissions are approved), restarted when their folder,
//! manifest, parts or limits change. Each is told where its widgets are and whether
//! they can be seen.

use crate::{dbus, screens};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use nyx_desktop::{Desktop, watch};
use nyx_desktop_widgets_core::{Layout, Placement, layout::split_part};
use nyx_plugin::{Kind, Manifest, Plugin, State, discovery, manifest::desktop_size, paths};
use nyx_plugin_host::{self as host, Handle, Report, Spec, frames::Buffers, limits::Limits};
use nyx_plugin_protocol::{
    API_VERSION, DesktopMonitor, DesktopState, DesktopWorkspace, Event, EventKind, Hello, HostMsg, Item, MenuEntry, MonitorInfo,
    Node, PluginMsg, ThemeInfo, Widget,
};
use nyx_plugin_view as view;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum RunState {
    Starting,
    Running { pid: u32, scoped: bool, sandboxed: bool },
    Crashed(String),
    Unavailable(String),
}

pub struct Running {
    pub plugin: Plugin,
    pub handle: Handle,
    pub parts: Vec<String>,
    settings: serde_json::Value,
    limits: Limits,
    /// Items in the order they first came.
    pub items: Vec<Item>,
    /// What a click (popover) and a right click (menu) open, by (item, monitor).
    pub popovers: HashMap<(String, Option<String>), Node>,
    pub menus: HashMap<(String, Option<String>), Vec<MenuEntry>>,
    pub frames: HashMap<(String, String), Buffers>,
    pub state: RunState,
    generation: u64,
    warned: HashSet<String>,
    /// What it was last told.
    widgets: Vec<Widget>,
    visible: HashMap<String, bool>,
    any_visible: Option<bool>,
}

pub struct Shared {
    pub app: gtk::Application,
    pub only: Option<String>,
    pub layout: RefCell<Layout>,
    pub desktop: RefCell<Arc<Desktop>>,
    pub plugins: RefCell<BTreeMap<String, Running>>,
    pub screens: RefCell<Vec<Rc<screens::Screen>>>,
    pub editing: Cell<bool>,
    /// Parts shown in the open **Add widget** list: their plugins run for the previews.
    pub previews: RefCell<Vec<(String, String)>>,
    /// The session is locked (logind's LockedHint): nothing can be seen.
    pub locked: Cell<bool>,
    css: RefCell<HashMap<String, gtk::CssProvider>>,
    user_css: gtk::CssProvider,
    generation: Cell<u64>,
    sync_pending: Cell<bool>,
    monitors: RefCell<Vec<gio::FileMonitor>>,
    plugin_monitors: RefCell<HashMap<String, gio::FileMonitor>>,
    changed_plugins: RefCell<HashSet<String>>,
    _dbus: RefCell<Option<dbus::Registration>>,
}

thread_local! {
    static SHARED: RefCell<Weak<Shared>> = const { RefCell::new(Weak::new()) };
}

pub fn debug(f: impl FnOnce() -> String) {
    if std::env::var_os("NYX_DW_DEBUG").is_some() {
        eprintln!("nyx-desktop-widgets: {}", f());
    }
}

pub fn layout_path() -> std::path::PathBuf {
    paths::layout_file(Kind::Desktop)
}

impl Shared {
    fn host_name(&self) -> &'static str {
        if self.only.is_some() { "desktop-test" } else { "desktop" }
    }

    /// A test copy never takes the keyboard or grabs input.
    pub fn test(&self) -> bool {
        self.only.is_some()
    }
}

pub fn start(app: &gtk::Application, only: Option<String>, edit: bool) {
    let layout = Layout::load(&layout_path()).unwrap_or_else(|e| {
        eprintln!("nyx-desktop-widgets: {e}; showing no widgets until it's fixed");
        Layout::default()
    });
    let s = Rc::new(Shared {
        app: app.clone(),
        only,
        layout: RefCell::new(layout),
        desktop: RefCell::new(Arc::new(Desktop::default())),
        plugins: RefCell::default(),
        screens: RefCell::default(),
        editing: Cell::new(false),
        previews: RefCell::default(),
        locked: Cell::new(false),
        css: RefCell::default(),
        user_css: gtk::CssProvider::new(),
        generation: Cell::new(0),
        sync_pending: Cell::new(false),
        monitors: RefCell::default(),
        plugin_monitors: RefCell::default(),
        changed_plugins: RefCell::default(),
        _dbus: RefCell::default(),
    });
    SHARED.with(|w| *w.borrow_mut() = Rc::downgrade(&s));
    std::mem::forget(app.hold());
    host::reap_stale(s.host_name());
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &s.user_css, gtk::STYLE_PROVIDER_PRIORITY_USER);
    }
    load_user_css(&s);

    let ws = Rc::downgrade(&s);
    std::mem::forget(watch::watch_local(move |u| {
        let Some(s) = ws.upgrade() else { return };
        *s.desktop.borrow_mut() = u.desktop.clone();
        if u.changes.monitors || s.screens.borrow().is_empty() {
            screens::rebuild(&s);
            let monitors = monitor_infos(&s);
            broadcast(&s, |_| Some(HostMsg::Monitors { monitors: monitors.clone() }));
            schedule_sync(&s);
        }
        if u.changes.windows || u.changes.workspaces || u.changes.active || u.changes.monitors {
            screens::update_visibility(&s);
        }
        if u.changes.workspaces || u.changes.active || u.changes.focus || u.changes.titles || u.changes.windows {
            let state = desktop_state(&u.desktop);
            broadcast(&s, |r| r.plugin.manifest.permissions.desktop.then(|| HostMsg::Desktop(state.clone())));
        }
    }));

    if let Some(display) = gdk::Display::default() {
        let ws = Rc::downgrade(&s);
        display.monitors().connect_items_changed(move |_, _, _, _| {
            let ws = ws.clone();
            glib::timeout_add_local_once(Duration::from_millis(300), move || {
                if let Some(s) = ws.upgrade() {
                    screens::rebuild(&s);
                    schedule_sync(&s);
                }
            });
        });
    }

    watch_files(&s);
    let ws = Rc::downgrade(&s);
    nyx_theme::on_change(move || {
        if let Some(s) = ws.upgrade() {
            let theme = theme_info();
            broadcast(&s, |_| Some(HostMsg::Theme { theme: theme.clone() }));
            screens::rerender_all(&s);
            // The text scale sizes widgets.
            screens::relayout_all(&s);
        }
    });

    if let Some(conn) = app.dbus_connection() {
        match dbus::serve(&conn, &s) {
            Ok(r) => *s._dbus.borrow_mut() = Some(r),
            Err(e) => eprintln!("nyx-desktop-widgets: couldn't serve DesktopWidgets1 on D-Bus: {e}"),
        }
        // A test copy is non-unique, so it doesn't own the app's name; give it its own.
        if s.only.is_some() {
            gio::bus_own_name_on_connection(
                &conn,
                &format!("{}.Test", crate::APP_ID),
                gio::BusNameOwnerFlags::NONE,
                |_, _| {},
                |_, name| eprintln!("nyx-desktop-widgets: {name} is taken; this test copy has no DesktopWidgets1"),
            );
        }
    }
    watch_lock(&s);
    sync_plugins(&s);
    if edit {
        let ws = Rc::downgrade(&s);
        glib::timeout_add_local_once(Duration::from_millis(400), move || {
            if let Some(s) = ws.upgrade() {
                screens::set_editing(&s, true);
            }
        });
    }
    // Everything else holds this weakly; it lives as long as the app.
    std::mem::forget(s);
}

/// Follow logind's LockedHint for this session: a locked screen shows no widgets.
fn watch_lock(s: &Rc<Shared>) {
    let ws = Rc::downgrade(s);
    gio::DBusProxy::for_bus(
        gio::BusType::System,
        gio::DBusProxyFlags::DO_NOT_AUTO_START,
        None,
        "org.freedesktop.login1",
        "/org/freedesktop/login1/session/auto",
        "org.freedesktop.login1.Session",
        gio::Cancellable::NONE,
        move |res| {
            let Ok(proxy) = res else { return };
            let read = |p: &gio::DBusProxy| p.cached_property("LockedHint").and_then(|v| v.get::<bool>()).unwrap_or(false);
            if let Some(s) = ws.upgrade() {
                s.locked.set(read(&proxy));
            }
            let ws2 = ws.clone();
            proxy.connect_local("g-properties-changed", false, move |args| {
                let (Some(s), Some(p)) = (ws2.upgrade(), args.first().and_then(|a| a.get::<gio::DBusProxy>().ok())) else {
                    return None;
                };
                let locked = read(&p);
                if s.locked.replace(locked) != locked {
                    debug(|| format!("session {}", if locked { "locked" } else { "unlocked" }));
                    screens::update_visibility(&s);
                }
                None
            });
            // Kept for as long as the app runs.
            std::mem::forget(proxy);
        },
    );
}

pub fn set_editing(on: bool) {
    if let Some(s) = SHARED.with(|w| w.borrow().upgrade()) {
        screens::set_editing(&s, on);
    }
}

// ---------- Files ----------

fn monitor_file(path: &std::path::Path, f: impl Fn() + 'static) -> Option<gio::FileMonitor> {
    let m = gio::File::for_path(path).monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE).ok()?;
    m.connect_changed(move |_, _, _, ev| {
        if matches!(
            ev,
            gio::FileMonitorEvent::ChangesDoneHint
                | gio::FileMonitorEvent::Created
                | gio::FileMonitorEvent::Deleted
                | gio::FileMonitorEvent::MovedIn
                | gio::FileMonitorEvent::Renamed
        ) {
            f();
        }
    });
    Some(m)
}

fn monitor_dir(path: &std::path::Path, f: impl Fn(Option<String>) + 'static) -> Option<gio::FileMonitor> {
    let _ = std::fs::create_dir_all(path);
    let m = gio::File::for_path(path).monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE).ok()?;
    m.connect_changed(move |_, file, _, _| f(file.basename().map(|b| b.to_string_lossy().into_owned())));
    Some(m)
}

fn watch_files(s: &Rc<Shared>) {
    let mut monitors = vec![];
    let ws = Rc::downgrade(s);
    monitors.extend(monitor_file(&layout_path(), move || {
        if let Some(s) = ws.upgrade() {
            reload_layout(&s);
        }
    }));
    let ws = Rc::downgrade(s);
    monitors.extend(monitor_file(&paths::state_file(), move || {
        if let Some(s) = ws.upgrade() {
            schedule_sync(&s);
        }
    }));
    let ws = Rc::downgrade(s);
    monitors.extend(monitor_file(&paths::config_home().join("nyx/desktop-widgets.css"), move || {
        if let Some(s) = ws.upgrade() {
            load_user_css(&s);
        }
    }));
    for (dir, _) in discovery::search_dirs() {
        let ws = Rc::downgrade(s);
        monitors.extend(monitor_dir(&dir, move |name| {
            let Some(s) = ws.upgrade() else { return };
            if !name.as_deref().is_some_and(|n| n.starts_with('.')) {
                schedule_sync(&s);
            }
        }));
    }
    *s.monitors.borrow_mut() = monitors;
}

/// Read the layout file again and show it.
pub fn reload_layout(s: &Rc<Shared>) {
    match Layout::load(&layout_path()) {
        Ok(l) => {
            if *s.layout.borrow() == l {
                return;
            }
            *s.layout.borrow_mut() = l;
            screens::relayout_all(s);
            schedule_sync(s);
        }
        Err(e) => eprintln!("nyx-desktop-widgets: {e}; keeping the layout as it was"),
    }
}

/// Change the layout file (keeping its comments) and show the result at once.
pub fn edit_layout(s: &Rc<Shared>, f: impl FnOnce(&mut toml_edit::DocumentMut) -> bool) -> bool {
    let path = layout_path();
    let mut doc = match nyx_plugin::layout::read(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("nyx-desktop-widgets: {e:#}");
            return false;
        }
    };
    if !f(&mut doc) {
        return false;
    }
    if let Err(e) = nyx_plugin::layout::write(&path, &doc) {
        eprintln!("nyx-desktop-widgets: {e:#}");
        return false;
    }
    reload_layout(s);
    true
}

fn load_user_css(s: &Shared) {
    let path = paths::config_home().join("nyx/desktop-widgets.css");
    s.user_css.load_from_string(&std::fs::read_to_string(path).unwrap_or_default());
}

/// Watch each running plugin's folder: an edit restarts it.
fn watch_plugin_dirs(s: &Rc<Shared>) {
    let running: Vec<(String, std::path::PathBuf)> =
        s.plugins.borrow().iter().map(|(id, r)| (id.clone(), r.plugin.dir.clone())).collect();
    let mut mons = s.plugin_monitors.borrow_mut();
    mons.retain(|id, _| running.iter().any(|(r, _)| r == id));
    for (id, dir) in running {
        if mons.contains_key(&id) {
            continue;
        }
        let ws = Rc::downgrade(s);
        let pid = id.clone();
        if let Some(m) = monitor_dir(&dir.canonicalize().unwrap_or(dir), move |name| {
            let Some(s) = ws.upgrade() else { return };
            if name.as_deref() == Some(discovery::ORIGIN_FILE) {
                return;
            }
            s.changed_plugins.borrow_mut().insert(pid.clone());
            schedule_sync(&s);
        }) {
            mons.insert(id, m);
        }
    }
}

pub fn schedule_sync(s: &Rc<Shared>) {
    if s.sync_pending.replace(true) {
        return;
    }
    let ws = Rc::downgrade(s);
    glib::timeout_add_local_once(Duration::from_millis(300), move || {
        if let Some(s) = ws.upgrade() {
            s.sync_pending.set(false);
            sync_plugins(&s);
        }
    });
}

// ---------- Plugins ----------

fn theme_info() -> ThemeInfo {
    ThemeInfo { light: nyx_theme::palette().light, name: nyx_theme::current_theme().id }
}

fn monitor_infos(s: &Shared) -> Vec<MonitorInfo> {
    s.desktop
        .borrow()
        .monitors
        .iter()
        .filter(|m| !m.disabled)
        .map(|m| {
            let (w, h) = m.logical_size();
            MonitorInfo { name: m.name.clone(), stable_id: m.stable_id(), width: w, height: h, scale: m.scale, focused: m.focused }
        })
        .collect()
}

fn desktop_state(d: &Desktop) -> DesktopState {
    let active = d.active_window.as_deref().and_then(|a| d.window(a));
    DesktopState {
        monitors: d
            .monitors
            .iter()
            .map(|m| DesktopMonitor { name: m.name.clone(), active_workspace: m.active_workspace, focused: m.focused })
            .collect(),
        workspaces: d
            .workspaces
            .iter()
            .filter(|w| w.is_user())
            .map(|w| DesktopWorkspace {
                id: w.id,
                name: w.name.clone(),
                monitor: w.monitor.clone(),
                windows: w.windows,
                urgent: d.windows.iter().any(|x| x.workspace == w.id && x.urgent),
                fullscreen: w.has_fullscreen,
            })
            .collect(),
        active_class: active.map(|w| w.class.clone()),
        active_title: active.map(|w| w.title.clone()),
    }
}

fn broadcast(s: &Shared, msg: impl Fn(&Running) -> Option<HostMsg>) {
    for r in s.plugins.borrow().values() {
        if let Some(m) = msg(r) {
            r.handle.send(m);
        }
    }
}

/// The size a placement shows at: its free size if the part is resizable, else the
/// size it names if the part offers it, else the part's first, grown with the suite's
/// text scale.
pub fn resolve_size(manifest: &Manifest, part: &str, p: &Placement) -> (String, (i32, i32)) {
    let found = manifest.parts(Kind::Desktop).into_iter().find(|x| x.id == part);
    if let Some(pt) = found.as_ref().filter(|pt| pt.resizable())
        && p.size.as_deref() == Some("free")
        && let (Some(w), Some(h)) = (p.width, p.height)
    {
        let (min, max) = pt.free_range();
        return ("free".into(), (w.clamp(min.0, max.0), h.clamp(min.1, max.1)));
    }
    let sizes: Vec<String> = found.map(|x| x.sizes()).unwrap_or_default().into_iter().filter(|s| s != "free").collect();
    let name = p
        .size
        .clone()
        .filter(|n| sizes.contains(n))
        .or_else(|| sizes.first().cloned())
        .unwrap_or_else(|| "medium".into());
    let (w, h) = desktop_size(&name).unwrap_or((336, 160));
    let k = view::text_scale();
    (name, ((w as f64 * k).round() as i32, (h as f64 * k).round() as i32))
}

/// Where previews in the **Add widget** list are drawn, as a monitor name.
pub const PREVIEW: &str = "preview";

/// The widgets placed from one plugin on the monitors shown.
fn widgets_for(s: &Shared, manifest: &Manifest, parts: &[String]) -> Vec<Widget> {
    let layout = s.layout.borrow();
    let mut out = vec![];
    for screen in s.screens.borrow().iter() {
        for (_, p) in layout.for_monitor(&screen.stable_id, &screen.connector) {
            let (plugin, part) = split_part(&p.part);
            if plugin != manifest.id || !parts.iter().any(|x| x == part) {
                continue;
            }
            let (size, (width, height)) = resolve_size(manifest, part, &p);
            let options = serde_json::to_value(&p.options).unwrap_or_default();
            let options = if options.as_object().is_some_and(|o| o.is_empty()) { serde_json::Value::Null } else { options };
            out.push(Widget { item: part.to_string(), monitor: screen.connector.clone(), size, width, height, options });
        }
    }
    drop(layout);
    for (plugin, part) in s.previews.borrow().iter() {
        if *plugin != manifest.id || !parts.contains(part) {
            continue;
        }
        let (size, (width, height)) = resolve_size(manifest, part, &Placement::new(plugin));
        out.push(Widget { item: part.clone(), monitor: PREVIEW.into(), size, width, height, options: serde_json::Value::Null });
    }
    out
}

/// Plugins with a widget on a monitor that's shown.
fn placed_plugins(s: &Shared) -> HashSet<String> {
    let layout = s.layout.borrow();
    let mut placed: HashSet<String> = s
        .screens
        .borrow()
        .iter()
        .flat_map(|sc| layout.for_monitor(&sc.stable_id, &sc.connector))
        .map(|(_, p)| split_part(&p.part).0.to_string())
        .collect();
    placed.extend(s.previews.borrow().iter().map(|(p, _)| p.clone()));
    placed
}

/// Start, restart and stop plugins to match the folders, `plugins.toml` and the layout.
pub fn sync_plugins(s: &Rc<Shared>) {
    let found = discovery::discover();
    for p in &found.problems {
        debug(|| format!("{}: {}", p.dir.display(), p.error));
    }
    let state = State::load();
    let changed: HashSet<String> = s.changed_plugins.take();
    let placed = placed_plugins(s);
    let wanted: Vec<Plugin> = found
        .plugins
        .into_iter()
        .filter(|p| p.manifest.has_kind(Kind::Desktop) && state.runs(p, Kind::Desktop) && placed.contains(&p.manifest.id))
        .collect();
    let layout = s.layout.borrow().clone();
    let stale: Vec<String> = s.plugins.borrow().keys().filter(|id| !wanted.iter().any(|p| p.manifest.id == **id)).cloned().collect();
    for id in stale {
        debug(|| format!("stopping {id}"));
        s.plugins.borrow_mut().remove(&id);
        if let Some(p) = s.css.borrow_mut().remove(&id) {
            remove_provider(p);
        }
    }
    for p in wanted {
        let id = p.manifest.id.clone();
        let parts = state.enabled_parts(&p.manifest, Kind::Desktop);
        let settings = layout.settings(&id, &p.manifest.settings);
        let mut limits = Limits::resolve(p.trust.is_trusted(), &p.manifest.limits, layout.limits(&id));
        // Widgets sit behind windows: 30 frames a second is plenty.
        limits.fps = limits.fps.min(30);
        let keep = {
            let mut plugins = s.plugins.borrow_mut();
            match plugins.get_mut(&id) {
                Some(r)
                    if r.plugin.manifest == p.manifest
                        && r.plugin.dir == p.dir
                        && r.plugin.trust == p.trust
                        && r.parts == parts
                        && r.limits == limits
                        && !changed.contains(&id)
                        && !matches!(r.state, RunState::Unavailable(_)) =>
                {
                    if r.settings != settings {
                        r.handle.send(HostMsg::Settings { settings: settings.clone() });
                        r.settings = settings.clone();
                    }
                    true
                }
                _ => false,
            }
        };
        if !keep {
            debug(|| format!("starting {id} ({})", p.trust.label()));
            spawn_plugin(s, p, parts, settings, limits);
        }
    }
    watch_plugin_dirs(s);
    screens::relayout_all(s);
    send_widgets(s);
}

/// Tell plugins whose widgets changed.
pub fn send_widgets(s: &Shared) {
    let mut plugins = s.plugins.borrow_mut();
    for r in plugins.values_mut() {
        let now = widgets_for(s, &r.plugin.manifest, &r.parts);
        if now != r.widgets {
            r.handle.send(HostMsg::Widgets { widgets: now.clone() });
            r.widgets = now;
        }
    }
}

/// Tell plugins which of their widgets can be seen: per item, and whether any can.
pub fn apply_visibility(s: &Shared, seen: &HashMap<(String, String), bool>) {
    let mut plugins = s.plugins.borrow_mut();
    for (id, r) in plugins.iter_mut() {
        let mut any = false;
        for part in r.parts.clone() {
            let Some(&v) = seen.get(&(id.clone(), part.clone())) else { continue };
            any |= v;
            if r.visible.insert(part.clone(), v) != Some(v) {
                debug(|| format!("{id}/{part} {}", if v { "visible" } else { "covered" }));
                r.handle.send(HostMsg::Visible { visible: v, item: Some(part) });
            }
        }
        if r.any_visible.replace(any) != Some(any) {
            r.handle.send(HostMsg::Visible { visible: any, item: None });
        }
    }
}

fn remove_provider(p: gtk::CssProvider) {
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_remove_provider_for_display(&display, &p);
    }
}

fn load_plugin_css(s: &Shared, p: &Plugin, limits: &Limits) {
    if let Some(old) = s.css.borrow_mut().remove(&p.manifest.id) {
        remove_provider(old);
    }
    let Ok(text) = std::fs::read_to_string(p.dir.join("style.css")) else { return };
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&view::css::scope(&text, &p.manifest.id, limits.infinite_animations));
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    }
    s.css.borrow_mut().insert(p.manifest.id.clone(), provider);
}

fn spawn_plugin(s: &Rc<Shared>, p: Plugin, parts: Vec<String>, settings: serde_json::Value, limits: Limits) {
    let id = p.manifest.id.clone();
    load_plugin_css(s, &p, &limits);
    let widgets = widgets_for(s, &p.manifest, &parts);
    let hello = Hello {
        api_version: API_VERSION,
        kind: Kind::Desktop.name().into(),
        plugin: id.clone(),
        items: parts.clone(),
        settings: settings.clone(),
        monitors: monitor_infos(s),
        theme: theme_info(),
        permissions: vec![],
        frame_socket: None,
        bar_height: None,
        widgets: widgets.clone(),
    };
    let spec = Spec { plugin: p.clone(), kind: Kind::Desktop, host: s.host_name().into(), hello, limits: limits.clone() };
    let (tx, rx) = async_channel::unbounded();
    let handle = host::spawn(spec, tx);
    let generation = s.generation.get() + 1;
    s.generation.set(generation);
    // Replacing an old run drops its handle, which stops it.
    s.plugins.borrow_mut().insert(
        id.clone(),
        Running {
            plugin: p,
            handle,
            parts,
            settings,
            limits,
            items: vec![],
            popovers: HashMap::new(),
            menus: HashMap::new(),
            frames: HashMap::new(),
            state: RunState::Starting,
            generation,
            warned: HashSet::new(),
            widgets,
            visible: HashMap::new(),
            any_visible: None,
        },
    );
    let ws = Rc::downgrade(s);
    glib::spawn_future_local(async move {
        while let Ok(report) = rx.recv().await {
            let Some(s) = ws.upgrade() else { return };
            let current = s.plugins.borrow().get(&id).is_some_and(|r| r.generation == generation);
            if !current {
                return;
            }
            on_report(&s, &id, report);
        }
    });
    // Its widgets' visibility is news to it.
    let ws = Rc::downgrade(s);
    glib::idle_add_local_once(move || {
        if let Some(s) = ws.upgrade() {
            screens::update_visibility(&s);
        }
    });
}

fn warn_once(s: &Shared, id: &str, what: String) {
    let mut plugins = s.plugins.borrow_mut();
    if let Some(r) = plugins.get_mut(id)
        && r.warned.insert(what.clone())
    {
        eprintln!("nyx-desktop-widgets: {id}: {what}");
    }
}

fn on_report(s: &Rc<Shared>, id: &str, report: Report) {
    match report {
        Report::Started { pid, scoped, sandboxed } => {
            debug(|| format!("{id} started (pid {pid}, scope {scoped}, sandbox {sandboxed})"));
            if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                r.state = RunState::Running { pid, scoped, sandboxed };
            }
        }
        Report::Message(m) => on_message(s, id, m),
        Report::Frames(b) => {
            if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                r.frames.insert((b.item.clone(), b.node.clone()), b);
            }
        }
        Report::Exited { status, stderr } => {
            eprintln!(
                "nyx-desktop-widgets: {id} stopped ({status}){}",
                if stderr.is_empty() { String::new() } else { format!(": {stderr}") }
            );
            if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                r.state = RunState::Crashed(status);
                r.items.clear();
                r.frames.clear();
                // A restarted run gets its visibility afresh.
                r.visible.clear();
                r.any_visible = None;
            }
            screens::refresh_plugin(s, id);
        }
        Report::Suspended { reason, crashed } => {
            let name = s.plugins.borrow().get(id).map(|r| r.plugin.manifest.name.clone()).unwrap_or_else(|| id.to_string());
            eprintln!("nyx-desktop-widgets: {id} is off: {reason}");
            if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                r.state = RunState::Unavailable(reason.clone());
                r.items.clear();
            }
            screens::refresh_plugin(s, id);
            if crashed {
                if let Ok(mut e) = nyx_plugin::state::Editor::open() {
                    e.suspend(id, &reason, &nyx_plugin::state::now_utc());
                    let _ = e.save();
                }
                if std::env::var_os("NYX_DW_NO_NOTIFY").is_some() {
                    return;
                }
                let _ = std::process::Command::new("notify-send")
                    .args([
                        "--app-name=Nyx Desktop Widgets",
                        "--icon=dialog-warning",
                        &format!("{name} was turned off"),
                        &format!("Its widgets stopped because {reason}. Turn it back on with: nyx plugin enable {id}"),
                    ])
                    .stdin(std::process::Stdio::null())
                    .spawn()
                    .map(|mut c| std::thread::spawn(move || c.wait()));
            }
        }
        Report::Warning(w) => warn_once(s, id, w),
    }
}

fn on_message(s: &Rc<Shared>, id: &str, msg: PluginMsg) {
    match msg {
        PluginMsg::Item(item) => {
            {
                let mut plugins = s.plugins.borrow_mut();
                let Some(r) = plugins.get_mut(id) else { return };
                let part = r.plugin.manifest.part_of(Kind::Desktop, &item.id);
                if !part.as_ref().is_some_and(|p| r.parts.contains(p)) {
                    let what = format!("item {} isn't one of the parts turned on ({})", item.id, r.parts.join(", "));
                    drop(plugins);
                    warn_once(s, id, what);
                    return;
                }
                match r.items.iter_mut().find(|i| i.id == item.id && i.monitor == item.monitor) {
                    Some(old) => *old = item,
                    None => r.items.push(item),
                }
            }
            screens::refresh_plugin(s, id);
        }
        PluginMsg::Patch(p) => {
            {
                let mut plugins = s.plugins.borrow_mut();
                let Some(r) = plugins.get_mut(id) else { return };
                for item in r.items.iter_mut().filter(|i| i.id == p.item && (p.monitor.is_none() || i.monitor == p.monitor)) {
                    let mut v = item.view.take().unwrap_or_else(|| view::simple(item));
                    for n in &p.nodes {
                        v.apply_patch(&n.id, &n.set);
                    }
                    item.view = Some(v);
                }
                for (k, node) in r.popovers.iter_mut() {
                    if k.0 == p.item {
                        for n in &p.nodes {
                            node.apply_patch(&n.id, &n.set);
                        }
                    }
                }
            }
            screens::patch(s, id, &p);
        }
        PluginMsg::Remove { item, monitor } => {
            if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                r.items.retain(|i| !(i.id == item && (monitor.is_none() || i.monitor == monitor)));
            }
            screens::refresh_plugin(s, id);
        }
        PluginMsg::Log { level, message } => eprintln!("[{id}] {level:?}: {message}"),
        PluginMsg::FrameReady { item, node, buffer } => {
            let Some(b) = s.plugins.borrow().get(id).and_then(|r| r.frames.get(&(item.clone(), node.clone())).cloned()) else { return };
            let Some(map) = b.maps.get(buffer as usize) else { return };
            screens::show_frame(s, id, &item, &node, map.bytes(), b.width, b.height, b.stride);
        }
        PluginMsg::Popover(p) => {
            let key = (p.item.clone(), p.monitor.clone());
            if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                match p.view {
                    Some(v) => {
                        r.popovers.insert(key, v);
                    }
                    None => {
                        r.popovers.remove(&key);
                    }
                }
            }
            screens::refresh_popover(s, id, &p.item);
        }
        PluginMsg::Menu(m) => {
            if let Some(r) = s.plugins.borrow_mut().get_mut(id) {
                r.menus.insert((m.item, m.monitor), m.entries);
            }
        }
        PluginMsg::FrameRequest { .. } => {}
    }
}

/// The item a widget shows: its monitor's copy, or the one for every monitor.
pub fn item_for(r: &Running, part: &str, connector: &str) -> Option<Item> {
    let of_part = |i: &&Item| r.plugin.manifest.part_of(Kind::Desktop, &i.id).as_deref() == Some(part);
    r.items
        .iter()
        .filter(of_part)
        .find(|i| i.monitor.as_deref() == Some(connector))
        .or_else(|| r.items.iter().filter(of_part).find(|i| i.monitor.is_none()))
        .cloned()
}

#[allow(clippy::too_many_arguments)]
pub fn send_event(
    s: &Shared,
    plugin: &str,
    item: &str,
    connector: &str,
    kind: EventKind,
    button: Option<u32>,
    control: Option<String>,
    value: Option<serde_json::Value>,
) {
    if let Some(r) = s.plugins.borrow().get(plugin) {
        r.handle.send(HostMsg::Event(Event {
            item: item.into(),
            kind,
            monitor: Some(connector.into()),
            button,
            direction: None,
            control,
            value,
            anchor: None,
        }));
    }
}

/// Run a command an item asks for, if the plugin may.
pub fn run_for(s: &Shared, plugin: &str, argv: &[String]) {
    let Some(p) = s.plugins.borrow().get(plugin).map(|r| r.plugin.clone()) else { return };
    if !host::allowed_command(&p, argv) {
        warn_once(s, plugin, format!("it asked to run {:?}, which isn't in its host_commands", argv.join(" ")));
        return;
    }
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).process_group(0);
    match cmd.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(e) => eprintln!("nyx-desktop-widgets: running {argv:?} for {plugin}: {e}"),
    }
}

/// For `DesktopWidgets1.ListWidgets`: the plugins, then every widget shown.
pub fn list(s: &Shared) -> serde_json::Value {
    let plugins = s.plugins.borrow();
    let state = State::load();
    let mut out = vec![];
    for p in discovery::discover().plugins.into_iter().filter(|p| p.manifest.has_kind(Kind::Desktop)) {
        let id = p.manifest.id.clone();
        let r = plugins.get(&id);
        let (memory, cpu) = r.and_then(|r| r.handle.usage()).map(|(m, c)| (Some(m), c)).unwrap_or((None, None));
        let status = match r.map(|r| &r.state) {
            Some(RunState::Running { .. }) => "running".to_string(),
            Some(RunState::Starting) => "starting".to_string(),
            Some(RunState::Crashed(why)) => format!("restarting ({why})"),
            Some(RunState::Unavailable(why)) => format!("off: {why}"),
            None if state.suspended.contains_key(&id) => format!("off: {}", state.suspended[&id].reason),
            None if !nyx_plugin::authorized(&p, &state) => "needs approval".to_string(),
            None if state.enabled_parts(&p.manifest, Kind::Desktop).is_empty() => "off".to_string(),
            None => "not placed".to_string(),
        };
        out.push(serde_json::json!({
            "id": id,
            "status": status,
            "pid": r.and_then(|r| r.handle.pid()),
            "sandboxed": r.map(|r| r.handle.sandboxed).unwrap_or(!p.trust.is_trusted()),
            "scoped": r.map(|r| r.handle.scoped).unwrap_or(false),
            "restarts": r.map(|r| r.handle.restarts()).unwrap_or(0),
            "memory_bytes": memory,
            "cpu_percent": cpu,
            "parts": r.map(|r| r.parts.clone()).unwrap_or_default(),
            "permissions": p.manifest.permissions.names(),
            "visible": r.and_then(|r| r.any_visible),
        }));
    }
    drop(plugins);
    serde_json::json!({ "plugins": out, "widgets": screens::list(s), "editing": s.editing.get() })
}

pub fn restart(s: &Rc<Shared>, id: &str) -> bool {
    let exists = s.plugins.borrow().contains_key(id);
    if exists {
        s.changed_plugins.borrow_mut().insert(id.to_string());
        sync_plugins(s);
    }
    exists
}

pub fn reload(s: &Rc<Shared>) {
    reload_layout(s);
    sync_plugins(s);
}
