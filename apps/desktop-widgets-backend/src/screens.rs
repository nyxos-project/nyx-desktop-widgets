//! The widgets on each monitor, worked out here and drawn by the interface: where each
//! sits (anchor and offset in the monitor's free area), its size, what its plugin shows,
//! whether windows cover it, and edit mode's changes to the layout file. The GTK host's
//! `screens.rs` did the same with widgets.

use crate::host::{self, OpenPopover, Shared};
use nyx_desktop_widgets_core::geometry::{self, Rect};
use nyx_desktop_widgets_core::layout::{self, split_part};
use nyx_desktop_widgets_core::{Anchor, Background, Placement, Scope};
use nyx_plugin::{Kind, State, discovery, paths};
use nyx_plugin_protocol::{EventKind, Item};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct Screen {
    pub connector: String,
    pub stable_id: String,
}

/// One widget as placed on a monitor.
struct Tile {
    scope: Scope,
    placement: Placement,
    plugin: String,
    part: String,
    size_name: String,
    size: (i32, i32),
    pos: (i32, i32),
    interactive: bool,
    available: bool,
    free: Option<((i32, i32), (i32, i32))>,
    on_workspace: bool,
    item: Option<Item>,
}

impl Tile {
    fn key(&self) -> String {
        if self.part == "main" { self.plugin.clone() } else { format!("{}/{}", self.plugin, self.part) }
    }

    /// What identifies it on its monitor: a part can be placed for every workspace and
    /// for one.
    fn id(&self) -> String {
        format!("{}|{}", self.scope.describe(), self.key())
    }

    fn has_item(&self) -> bool {
        self.available && self.item.as_ref().is_some_and(|i| i.visible)
    }

    fn shown(&self, editing: bool) -> bool {
        (editing || self.has_item()) && self.on_workspace
    }
}

pub fn rebuild(s: &Rc<Shared>) {
    let desktop = s.desktop.borrow().clone();
    let mut screens = vec![];
    for m in desktop.monitors.iter().filter(|m| !m.disabled) {
        match &s.only {
            Some(o) if *o != m.name => continue,
            // Headless outputs belong to tests and remote sessions.
            None if m.is_virtual() => continue,
            _ => {}
        }
        screens.push(Screen { connector: m.name.clone(), stable_id: m.stable_id() });
    }
    *s.screens.borrow_mut() = screens;
    host::send_widgets(s);
    host::publish(s);
    update_visibility(s);
}

/// The monitor's active workspace: its id and its name.
pub fn active_workspace(s: &Shared, connector: &str) -> Vec<String> {
    let desktop = s.desktop.borrow();
    let Some(m) = desktop.monitor(connector) else { return vec![] };
    let mut keys = vec![m.active_workspace.to_string()];
    if let Some(w) = desktop.workspaces.iter().find(|w| w.id == m.active_workspace) {
        keys.push(w.name.clone());
    }
    keys
}

/// The monitor's room for widgets (what the bar and other panels leave), and where it
/// starts on the desktop.
fn area(s: &Shared, connector: &str) -> ((i32, i32), (i32, i32)) {
    let desktop = s.desktop.borrow();
    let Some(m) = desktop.monitor(connector) else { return ((1280, 800), (0, 0)) };
    let (w, h) = m.logical_size();
    let [l, t, r, b] = m.reserved;
    (((w - l - r).max(1), (h - t - b).max(1)), (m.x + l, m.y + t))
}

fn tiles(s: &Shared, sc: &Screen) -> Vec<Tile> {
    let placements = s.layout.borrow().for_monitor(&sc.stable_id, &sc.connector);
    let keys = active_workspace(s, &sc.connector);
    let active: Vec<&str> = keys.iter().map(String::as_str).collect();
    let shown = layout::shown_on(&placements, &active);
    let (room, _) = area(s, &sc.connector);
    let plugins = s.plugins.borrow();
    placements
        .into_iter()
        .enumerate()
        .map(|(i, (scope, p))| {
            let (plugin, part) = split_part(&p.part);
            let (plugin, part) = (plugin.to_string(), part.to_string());
            let running = plugins.get(&plugin).filter(|r| r.parts.contains(&part));
            let (available, size_name, size, interactive, free, item) = match running {
                Some(r) => {
                    let (name, px) = host::resolve_size(&r.plugin.manifest, &part, &p);
                    let found = r.plugin.manifest.parts(Kind::Desktop).into_iter().find(|x| x.id == part);
                    let interactive = found.as_ref().is_some_and(|x| x.interactive());
                    let free = found.filter(|x| x.resizable()).map(|x| x.free_range());
                    (true, name, px, interactive, free, host::item_for(r, &part, &sc.connector))
                }
                None => {
                    let name = p.size.clone().unwrap_or_else(|| "medium".into());
                    let px = match (p.width, p.height) {
                        (Some(w), Some(h)) if name == "free" => (w, h),
                        _ => nyx_plugin::manifest::desktop_size(&name).unwrap_or((336, 160)),
                    };
                    (false, name, px, false, None, None)
                }
            };
            let pos = geometry::place(room, size, p.anchor, (p.offset[0], p.offset[1]));
            Tile {
                scope,
                placement: p,
                plugin,
                part,
                size_name,
                size,
                pos,
                interactive,
                available,
                free,
                on_workspace: shown.contains(&i),
                item,
            }
        })
        .collect()
}

/// A view's file paths as files on this side of the sandbox; anything a third-party
/// plugin may not show is dropped.
fn resolve_paths(v: &mut Value, plugin_dir: &Path, storage_dir: &Path, trusted: bool) {
    let resolve = |p: &str| -> Option<PathBuf> {
        if p.contains("..") {
            return None;
        }
        if let Some(rest) = p.strip_prefix("/plugin/") {
            return Some(plugin_dir.join(rest));
        }
        if let Some(rest) = p.strip_prefix("/storage/") {
            return Some(storage_dir.join(rest));
        }
        if Path::new(p).is_absolute() {
            return trusted.then(|| PathBuf::from(p));
        }
        Some(plugin_dir.join(p))
    };
    match v {
        Value::Object(m) => {
            let node = m.get("node").and_then(Value::as_str).unwrap_or("").to_string();
            if node == "image"
                && let Some(Value::String(p)) = m.get("path").cloned()
            {
                match resolve(&p) {
                    Some(r) => m.insert("path".into(), Value::String(r.display().to_string())),
                    None => m.remove("path"),
                };
            }
            if node == "icon"
                && let Some(Value::String(n)) = m.get("name").cloned()
                && n.contains('/')
            {
                match resolve(&n) {
                    Some(r) => m.insert("name".into(), Value::String(r.display().to_string())),
                    None => m.insert("name".into(), Value::String("image-missing-symbolic".into())),
                };
            }
            for (_, c) in m.iter_mut() {
                resolve_paths(c, plugin_dir, storage_dir, trusted);
            }
        }
        Value::Array(a) => {
            for c in a {
                resolve_paths(c, plugin_dir, storage_dir, trusted);
            }
        }
        _ => {}
    }
}

/// An item's view for the interface, with its paths resolved and its revision.
fn view_json(s: &Shared, plugin: &str, item: &Item) -> Value {
    let plugins = s.plugins.borrow();
    let Some(r) = plugins.get(plugin) else { return Value::Null };
    let node = item.view.clone().unwrap_or_else(|| host::simple(item));
    let mut v = serde_json::to_value(&node).unwrap_or(Value::Null);
    resolve_paths(&mut v, &r.plugin.dir, &paths::storage_dir(plugin), r.plugin.trust.is_trusted());
    let rev = r.revs.get(&(item.id.clone(), item.monitor.clone())).copied().unwrap_or(0);
    json!({"id": item.id, "view": v, "rev": rev, "tooltip": item.tooltip, "class": item.class})
}

fn part_sizes(s: &Shared, plugin: &str, part: &str) -> Vec<String> {
    let from = |m: &nyx_plugin::Manifest| m.parts(Kind::Desktop).into_iter().find(|x| x.id == part).map(|x| x.sizes());
    if let Some(sizes) = s.plugins.borrow().get(plugin).and_then(|r| from(&r.plugin.manifest)) {
        return sizes;
    }
    discovery::discover().get(plugin).and_then(|p| from(&p.manifest)).unwrap_or_else(|| vec!["medium".into()])
}

/// What a widget shows when its plugin has nothing for it (edit mode shows these).
fn stand_in(s: &Shared, t: &Tile) -> Value {
    if !t.available {
        let name = discovery::discover().get(&t.plugin).map(|p| p.manifest.name.clone()).unwrap_or_else(|| t.key());
        let why = if s.plugins.borrow().contains_key(&t.plugin) { "this part is off" } else { "it isn't installed or turned on" };
        return json!({"title": format!("{name} isn't available"), "sub": why});
    }
    let name = s
        .plugins
        .borrow()
        .get(&t.plugin)
        .map(|r| {
            let part = r.plugin.manifest.parts(Kind::Desktop).into_iter().find(|p| p.id == t.part).map(|p| p.name);
            match part {
                Some(p) if p != r.plugin.manifest.name => format!("{} · {p}", r.plugin.manifest.name),
                _ => r.plugin.manifest.name.clone(),
            }
        })
        .unwrap_or_else(|| t.key());
    json!({"title": name, "sub": "Nothing to show right now"})
}

/// Everything the interface draws.
pub fn state_json(s: &Shared) -> Value {
    let editing = s.editing.get();
    let screens: Vec<Value> = s
        .screens
        .borrow()
        .iter()
        .map(|sc| {
            let (room, _) = area(s, &sc.connector);
            let keys = active_workspace(s, &sc.connector);
            let tiles: Vec<Value> = tiles(s, sc)
                .iter()
                .map(|t| {
                    let item = t.item.as_ref().filter(|_| t.has_item()).map(|i| view_json(s, &t.plugin, i)).unwrap_or(Value::Null);
                    let ws_tip = if t.scope.workspace().is_some() {
                        "Only on this workspace: click to show it on every workspace".to_string()
                    } else {
                        format!("On every workspace: click to keep it to workspace {}", keys.last().cloned().unwrap_or_default())
                    };
                    json!({
                        "id": t.id(),
                        "key": t.key(),
                        "plugin": t.plugin,
                        "part": t.part,
                        "workspace": t.scope.workspace(),
                        "ws_tip": ws_tip,
                        "anchor": t.placement.anchor.name(),
                        "x": t.pos.0, "y": t.pos.1, "w": t.size.0, "h": t.size.1,
                        "size": t.size_name,
                        "background": t.placement.background.name(),
                        "interactive": t.interactive,
                        "available": t.available,
                        "free": t.free.map(|((a, b), (c, d))| [a, b, c, d]),
                        "shown": t.shown(editing),
                        "item": item,
                        "stand_in": if t.has_item() { Value::Null } else { stand_in(s, t) },
                        "class": format!("part-{}", t.part),
                    })
                })
                .collect();
            json!({"connector": sc.connector, "stable_id": sc.stable_id, "area": [room.0, room.1], "tiles": tiles})
        })
        .collect();
    let popover = s.popover.borrow().clone().and_then(|p| {
        let plugins = s.plugins.borrow();
        let r = plugins.get(&p.plugin)?;
        let node = r
            .popovers
            .get(&(p.item.clone(), Some(p.connector.clone())))
            .or_else(|| r.popovers.get(&(p.item.clone(), None)))?
            .clone();
        let mut v = serde_json::to_value(&node).unwrap_or(Value::Null);
        resolve_paths(&mut v, &r.plugin.dir, &paths::storage_dir(&p.plugin), r.plugin.trust.is_trusted());
        let key = if p.part == "main" { p.plugin.clone() } else { format!("{}/{}", p.plugin, p.part) };
        Some(json!({"connector": p.connector, "plugin": p.plugin, "key": key, "item": p.item, "view": v, "rev": s.rev.get()}))
    });
    json!({
        "editing": editing,
        "test": s.test(),
        "screens": screens,
        "gallery": gallery_json(s),
        "popover": popover,
    })
}

// ---------- Covered by windows ----------

/// Which widgets windows cover, per monitor: the windows on its shown workspaces, or
/// everything when one is fullscreen or the screen is locked. Plugins hear which of
/// theirs can be seen.
pub fn update_visibility(s: &Rc<Shared>) {
    let desktop = s.desktop.borrow().clone();
    let editing = s.editing.get();
    let locked = s.locked.get();
    let mut seen: HashMap<(String, String), bool> = HashMap::new();
    for sc in s.screens.borrow().iter() {
        let Some(m) = desktop.monitor(&sc.connector) else { continue };
        let shown_ws: Vec<i64> = [Some(m.active_workspace), m.special_workspace].into_iter().flatten().collect();
        let fullscreen = desktop.workspaces.iter().any(|w| shown_ws.contains(&w.id) && w.has_fullscreen);
        let windows: Vec<Rect> = desktop
            .windows
            .iter()
            .filter(|w| shown_ws.contains(&w.workspace) && !w.hidden)
            .map(|w| Rect::new(w.at[0], w.at[1], w.size[0], w.size[1]))
            .collect();
        let (_, origin) = area(s, &sc.connector);
        for t in tiles(s, sc) {
            let rect = Rect::new(t.pos.0, t.pos.1, t.size.0, t.size.1).translate(origin.0, origin.1);
            let covered = locked || fullscreen || geometry::covered(&rect, &windows);
            let v = t.shown(editing) && !covered;
            let e = seen.entry((t.plugin.clone(), t.part.clone())).or_insert(false);
            *e |= v;
        }
    }
    host::apply_visibility(s, &seen);
}

// ---------- From the interface ----------

fn find(s: &Shared, connector: &str, id: &str) -> Option<Tile> {
    let screens = s.screens.borrow();
    let sc = screens.iter().find(|sc| sc.connector == connector || sc.stable_id == connector)?;
    tiles(s, sc).into_iter().find(|t| t.id() == id || (t.key() == id && t.on_workspace))
}

pub fn has_tile(s: &Shared, connector: &str, id: &str) -> bool {
    find(s, connector, id).is_some()
}

pub fn set_editing(s: &Rc<Shared>, on: bool) {
    if s.editing.replace(on) == on {
        return;
    }
    host::debug(|| format!("edit mode {}", if on { "on" } else { "off" }));
    if !on {
        close_gallery(s);
    }
    host::publish(s);
    update_visibility(s);
}

/// A click on a widget (not in edit mode): the command it asks for, its popover, its
/// menu (returned for the interface to show), or an event.
pub fn click(s: &Rc<Shared>, connector: &str, id: &str, button: u32) -> Value {
    if s.editing.get() {
        return Value::Null;
    }
    let Some(t) = find(s, connector, id) else { return Value::Null };
    let (cmd, item_id, has_popover, menu) = {
        let plugins = s.plugins.borrow();
        let Some(r) = plugins.get(&t.plugin) else { return Value::Null };
        let Some(i) = host::item_for(r, &t.part, connector) else { return Value::Null };
        let cmd = match button {
            1 => i.on_click.clone(),
            2 => i.on_middle_click.clone(),
            3 => i.on_right_click.clone(),
            _ => None,
        };
        let key = |m: Option<String>| (i.id.clone(), m);
        let has_popover = r.popovers.contains_key(&key(Some(connector.to_string()))) || r.popovers.contains_key(&key(None));
        let menu = r.menus.get(&key(Some(connector.to_string()))).or_else(|| r.menus.get(&key(None))).cloned();
        (cmd, i.id.clone(), has_popover, menu)
    };
    if let Some(c) = cmd {
        host::run_for(s, &t.plugin, &c);
        return Value::Null;
    }
    if button == 1 && has_popover {
        *s.popover.borrow_mut() =
            Some(OpenPopover { connector: connector.into(), plugin: t.plugin.clone(), part: t.part.clone(), item: item_id });
        host::publish(s);
        host::send_event(s, &t.plugin, &t.part, connector, EventKind::PopoverOpen, Some(1), None, None);
        return Value::Null;
    }
    if button == 3
        && let Some(entries) = menu
    {
        return json!({"menu": entries, "item": item_id});
    }
    host::send_event(s, &t.plugin, &t.part, connector, EventKind::Click, Some(button), None, None);
    Value::Null
}

pub fn close_popover(s: &Rc<Shared>) {
    if let Some(p) = s.popover.borrow_mut().take() {
        host::send_event(s, &p.plugin, &p.part, &p.connector, EventKind::PopoverClose, None, None, None);
    }
    host::publish(s);
}

/// An event from a widget's view (or its popover's): a control used, a menu entry.
pub fn view_event(s: &Rc<Shared>, connector: &str, id: &str, kind: &str, control: Option<String>, value: Option<Value>) {
    let Some(t) = find(s, connector, id) else { return };
    let Ok(kind) = serde_json::from_value::<EventKind>(Value::String(kind.into())) else { return };
    host::send_event(s, &t.plugin, &t.part, connector, kind, None, control, value);
}

/// Dropped at `pos` (top left, in the area): the anchor follows the third of the screen
/// its centre is in.
pub fn move_to(s: &Rc<Shared>, connector: &str, id: &str, pos: (i32, i32)) -> bool {
    let Some(t) = find(s, connector, id) else { return false };
    let (room, _) = area(s, connector);
    let (anchor, offset) = geometry::locate(room, t.size, pos);
    write_position(s, &t, anchor, [offset.0, offset.1])
}

fn write_position(s: &Rc<Shared>, t: &Tile, anchor: Anchor, offset: [i32; 2]) -> bool {
    let (scope, key) = (t.scope.clone(), t.key());
    host::edit_layout(s, |doc| layout::set_position(doc, &scope, &key, anchor, offset))
}

/// A free size from edit mode's grip: on the grid, within the part's range.
pub fn resize_free(s: &Rc<Shared>, connector: &str, id: &str, w: i32, h: i32) -> bool {
    let Some(t) = find(s, connector, id) else { return false };
    let ((min_w, min_h), (max_w, max_h)) = t.free.unwrap_or(((160, 120), (672, 504)));
    let (w, h) = (geometry::snap(w).clamp(min_w, max_w), geometry::snap(h).clamp(min_h, max_h));
    let (scope, key) = (t.scope.clone(), t.key());
    host::edit_layout(s, |doc| layout::set_free_size(doc, &scope, &key, w, h))
}

/// The next size the part offers.
pub fn cycle_size(s: &Rc<Shared>, connector: &str, id: &str) -> bool {
    let Some(t) = find(s, connector, id) else { return false };
    let sizes: Vec<String> = part_sizes(s, &t.plugin, &t.part).into_iter().filter(|x| x != "free").collect();
    let next = sizes.iter().position(|x| *x == t.size_name).map(|i| sizes[(i + 1) % sizes.len()].clone());
    let Some(next) = next.or_else(|| sizes.first().cloned()) else { return false };
    let (scope, key) = (t.scope.clone(), t.key());
    host::edit_layout(s, |doc| layout::set_size(doc, &scope, &key, &next))
}

pub fn toggle_background(s: &Rc<Shared>, connector: &str, id: &str) -> bool {
    let Some(t) = find(s, connector, id) else { return false };
    let next = if t.placement.background == Background::Card { Background::None } else { Background::Card };
    let (scope, key) = (t.scope.clone(), t.key());
    host::edit_layout(s, |doc| layout::set_background(doc, &scope, &key, next))
}

/// Keep it to the monitor's workspace (by its name, which is its id unless it was
/// named), or back on every workspace.
pub fn toggle_workspace(s: &Rc<Shared>, connector: &str, id: &str) -> bool {
    let Some(t) = find(s, connector, id) else { return false };
    let keys = active_workspace(s, connector);
    let from = t.scope.clone();
    let to = if from.workspace().is_some() { from.with_workspace(None) } else { from.with_workspace(keys.last().map(String::as_str)) };
    let key = t.key();
    host::edit_layout(s, |doc| layout::move_scope(doc, &from, &to, &key))
}

pub fn remove(s: &Rc<Shared>, connector: &str, id: &str) -> bool {
    let Some(t) = find(s, connector, id) else { return false };
    let (scope, key) = (t.scope.clone(), t.key());
    host::edit_layout(s, |doc| layout::remove(doc, &scope, &key))
}

// ---------- Add widget ----------

/// Every turned-on desktop part not on this monitor yet, with its plugin's preview,
/// then the installed ones that aren't turned on.
fn gallery_json(s: &Shared) -> Value {
    let Some(connector) = s.gallery.borrow().clone() else { return Value::Null };
    let state = State::load();
    let on_screen: Vec<String> = {
        let screens = s.screens.borrow();
        screens.iter().find(|sc| sc.connector == connector).map(|sc| tiles(s, sc).iter().map(Tile::key).collect()).unwrap_or_default()
    };
    let mut entries = vec![];
    let mut off = vec![];
    for p in discovery::discover().plugins.into_iter().filter(|p| p.manifest.has_kind(Kind::Desktop)) {
        let enabled = state.enabled_parts(&p.manifest, Kind::Desktop);
        let usable = nyx_plugin::authorized(&p, &state);
        for part in p.manifest.parts(Kind::Desktop) {
            let key = if part.id == "main" { p.manifest.id.clone() } else { format!("{}/{}", p.manifest.id, part.id) };
            if !(usable && enabled.contains(&part.id)) {
                off.push(p.manifest.id.clone());
                continue;
            }
            if on_screen.contains(&key) {
                continue;
            }
            let name = if part.name == p.manifest.name { part.name.clone() } else { format!("{} · {}", p.manifest.name, part.name) };
            let size = part.sizes().into_iter().find(|x| x != "free").unwrap_or_else(|| "medium".into());
            let (_, px) = host::resolve_size(&p.manifest, &part.id, &Placement::new(&key));
            let item = {
                let plugins = s.plugins.borrow();
                plugins.get(&p.manifest.id).and_then(|r| host::item_for(r, &part.id, host::PREVIEW))
            };
            let preview = item.as_ref().filter(|i| i.visible).map(|i| view_json(s, &p.manifest.id, i)).unwrap_or(Value::Null);
            entries.push(json!({
                "key": key, "plugin": p.manifest.id, "name": name, "description": part.description,
                "size": size, "w": px.0, "h": px.1, "item": preview, "started": item.is_some(),
            }));
        }
    }
    off.sort();
    off.dedup();
    json!({"connector": connector, "entries": entries, "off": off})
}

/// The **Add widget** list opened: the plugins behind its previews run while it's open.
pub fn open_gallery(s: &Rc<Shared>, connector: &str) {
    *s.gallery.borrow_mut() = Some(connector.to_string());
    let state = State::load();
    let mut previews = vec![];
    for p in discovery::discover().plugins.into_iter().filter(|p| p.manifest.has_kind(Kind::Desktop)) {
        let enabled = state.enabled_parts(&p.manifest, Kind::Desktop);
        if !nyx_plugin::authorized(&p, &state) {
            continue;
        }
        for part in p.manifest.parts(Kind::Desktop) {
            if enabled.contains(&part.id) {
                previews.push((p.manifest.id.clone(), part.id.clone()));
            }
        }
    }
    *s.previews.borrow_mut() = previews;
    host::sync_plugins(s);
    host::publish(s);
}

pub fn close_gallery(s: &Rc<Shared>) {
    if s.gallery.borrow_mut().take().is_some() {
        s.previews.borrow_mut().clear();
        host::schedule_sync(s);
        host::publish(s);
    }
}

/// Put a part on this monitor, top left, below what's there already.
pub fn add_widget(s: &Rc<Shared>, connector: &str, key: &str, size: &str) {
    let Some((stable_id, below)) = s.screens.borrow().iter().find(|sc| sc.connector == connector).map(|sc| {
        (sc.stable_id.clone(), tiles(s, sc).iter().filter(|t| t.placement.anchor == Anchor::TopLeft).count() as i32)
    }) else {
        return;
    };
    let scope = Scope::Monitor(s.layout.borrow().monitor_key(&stable_id, connector));
    let mut p = Placement::new(key);
    p.size = Some(size.to_string());
    p.offset = [48, geometry::snap(48 + below * 184)];
    host::edit_layout(s, |doc| {
        layout::add(doc, &scope, &p);
        true
    });
    close_gallery(s);
    host::schedule_sync(s);
}

// ---------- For D-Bus ----------

pub fn move_widget(s: &Rc<Shared>, part: &str, monitor: &str, anchor: &str, offset: [i32; 2]) -> bool {
    let (Some(t), Some(anchor)) = (find(s, monitor, part), Anchor::parse(anchor)) else { return false };
    write_position(s, &t, anchor, [geometry::snap(offset[0]), geometry::snap(offset[1])])
}

pub fn resize_widget(s: &Rc<Shared>, part: &str, monitor: &str, size: &str) -> bool {
    let Some(t) = find(s, monitor, part) else { return false };
    if !part_sizes(s, &t.plugin, &t.part).iter().any(|x| x == size) {
        return false;
    }
    let scope = t.scope.clone();
    host::edit_layout(s, |doc| layout::set_size(doc, &scope, part, size))
}

pub fn list(s: &Shared) -> Value {
    let editing = s.editing.get();
    let mut out = vec![];
    for sc in s.screens.borrow().iter() {
        for t in tiles(s, sc) {
            out.push(json!({
                "monitor": sc.connector,
                "stable_id": sc.stable_id,
                "part": t.key(),
                "scope": t.scope.describe(),
                "on_workspace": t.on_workspace,
                "anchor": t.placement.anchor.name(),
                "offset": t.placement.offset,
                "size": t.size_name,
                "background": t.placement.background.name(),
                "interactive": t.interactive,
                "available": t.available,
                "shown": t.shown(editing),
                "bounds": [t.pos.0, t.pos.1, t.size.0, t.size.1],
            }));
        }
    }
    Value::Array(out)
}
