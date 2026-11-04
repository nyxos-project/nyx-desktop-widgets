//! One layer-shell surface per monitor, on the bottom layer below every window, and the
//! widgets on it. Clicks pass through to the desktop except on interactive widgets;
//! in edit mode the surface takes them all, for dragging widgets around.

use crate::host::{self, Shared};
use gtk::prelude::*;
use gtk::{gdk, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use nyx_desktop_widgets_core::geometry::{self, Rect, Side};
use nyx_desktop_widgets_core::layout::{self, split_part};
use nyx_desktop_widgets_core::{Anchor, Background, Placement, Scope};
use nyx_plugin::{Kind, State, discovery};
use nyx_plugin_protocol::{EventKind, Item, MenuEntry, Patch};
use nyx_plugin_view::{self as view, Rendered, ViewEvent};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

pub struct Tile {
    pub plugin: String,
    /// The part within the plugin.
    pub part: String,
    scope: RefCell<Scope>,
    placement: RefCell<Placement>,
    size_name: RefCell<String>,
    size: Cell<(i32, i32)>,
    interactive: Cell<bool>,
    /// Its plugin runs with this part on; otherwise a placeholder in edit mode.
    available: Cell<bool>,
    /// What the plugin sent for it is visible.
    has_item: Cell<bool>,
    /// Not covered by windows.
    seen: Cell<bool>,
    /// The workspace it belongs to (none: every one), and whether it shows on the
    /// monitor's active workspace.
    workspace: Option<String>,
    on_workspace: Cell<bool>,
    root: gtk::Overlay,
    card: gtk::Box,
    /// Keeps what the plugin draws to the widget's size.
    clip: gtk::ScrolledWindow,
    shield: gtk::Box,
    chrome: gtk::Box,
    size_button: gtk::Button,
    bg_button: gtk::Button,
    ws_button: gtk::Button,
    /// Edit mode's handle for any size, on resizable parts: (min, max) when it is.
    grip: gtk::Box,
    free: Cell<Option<FreeRange>>,
    resize_start: Cell<(i32, i32)>,
    popover: RefCell<Option<(gtk::Popover, Rc<Rendered>)>>,
    menu: RefCell<Option<gtk::PopoverMenu>>,
    rendered: RefCell<Option<Rc<Rendered>>>,
    last: RefCell<Option<Item>>,
    /// Which stand-in it shows when the plugin has nothing for it.
    stand_in: Cell<StandIn>,
    drag_start: Cell<(i32, i32)>,
}

impl Tile {
    fn key(&self) -> String {
        if self.part == "main" { self.plugin.clone() } else { format!("{}/{}", self.plugin, self.part) }
    }

    fn shown(&self, editing: bool) -> bool {
        (editing || (self.available.get() && self.has_item.get())) && self.on_workspace.get()
    }
}

/// x, y, width, height.
type Region = (i32, i32, i32, i32);

/// A resizable part's smallest and largest size.
type FreeRange = ((i32, i32), (i32, i32));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StandIn {
    None,
    Unavailable,
    Empty,
}

pub struct Screen {
    pub connector: String,
    pub stable_id: String,
    gmon: gdk::Monitor,
    window: gtk::Window,
    overlay: gtk::Overlay,
    toolbar: gtk::Box,
    add: gtk::MenuButton,
    tiles: RefCell<Vec<Rc<Tile>>>,
    /// The input region last set, and the size it was set for: setting it lays the
    /// surface out again, so only a change sets it.
    input: RefCell<Option<Vec<Region>>>,
    size: Cell<(i32, i32)>,
}

// ---------- Screens ----------

pub fn rebuild(s: &Rc<Shared>) {
    let desktop = s.desktop.borrow().clone();
    let Some(display) = gdk::Display::default() else { return };
    let gmons: Vec<gdk::Monitor> = display.monitors().iter::<gdk::Monitor>().flatten().collect();
    let old: Vec<Rc<Screen>> = s.screens.borrow_mut().drain(..).collect();
    let mut screens = vec![];
    for m in desktop.monitors.iter().filter(|m| !m.disabled) {
        match &s.only {
            Some(o) if *o != m.name => continue,
            // Headless outputs belong to tests and remote sessions.
            None if m.is_virtual() => continue,
            _ => {}
        }
        let Some(gmon) = gmons.iter().find(|g| g.connector().as_deref() == Some(m.name.as_str())) else { continue };
        if let Some(sc) = old.iter().find(|sc| sc.connector == m.name && sc.gmon == *gmon) {
            screens.push(sc.clone());
            continue;
        }
        host::debug(|| format!("widgets on {}", m.name));
        screens.push(new_screen(s, &m.name, &m.stable_id(), gmon));
    }
    for sc in old.iter().filter(|o| !screens.iter().any(|k| Rc::ptr_eq(k, o))) {
        sc.window.destroy();
    }
    *s.screens.borrow_mut() = screens;
    relayout_all(s);
}

fn new_screen(s: &Rc<Shared>, connector: &str, stable_id: &str, gmon: &gdk::Monitor) -> Rc<Screen> {
    let window = gtk::Window::new();
    window.set_application(Some(&s.app));
    window.add_css_class("nyx-desktop-widgets");
    window.set_decorated(false);
    window.init_layer_shell();
    window.set_namespace(Some("nyx-desktop-widgets"));
    window.set_layer(Layer::Bottom);
    window.set_monitor(Some(gmon));
    window.set_keyboard_mode(KeyboardMode::None);
    for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(e, true);
    }
    // Zero: stay out of the bar's space, and take none.
    window.set_exclusive_zone(0);

    let overlay = gtk::Overlay::new();
    let base = gtk::Box::new(gtk::Orientation::Vertical, 0);
    base.add_css_class("dw-base");
    base.set_hexpand(true);
    base.set_vexpand(true);
    overlay.set_child(Some(&base));

    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    toolbar.add_css_class("dw-toolbar");
    toolbar.set_halign(gtk::Align::Center);
    toolbar.set_valign(gtk::Align::Start);
    toolbar.set_margin_top(24);
    toolbar.set_visible(false);
    let hint = gtk::Label::new(Some("Drag widgets to move them"));
    hint.add_css_class("dw-hint");
    let add = gtk::MenuButton::new();
    add.set_label("Add widget");
    add.add_css_class("dw-add");
    let done = gtk::Button::with_label("Done");
    done.add_css_class("suggested-action");
    toolbar.append(&hint);
    toolbar.append(&add);
    toolbar.append(&done);
    overlay.add_overlay(&toolbar);
    window.set_child(Some(&overlay));

    let sc = Rc::new(Screen {
        connector: connector.into(),
        stable_id: stable_id.into(),
        gmon: gmon.clone(),
        window: window.clone(),
        overlay,
        toolbar,
        add: add.clone(),
        tiles: RefCell::default(),
        input: RefCell::default(),
        size: Cell::new((0, 0)),
    });

    let ws = Rc::downgrade(s);
    done.connect_clicked(move |_| {
        if let Some(s) = ws.upgrade() {
            set_editing(&s, false);
        }
    });
    let (ws, wsc) = (Rc::downgrade(s), Rc::downgrade(&sc));
    add.set_create_popup_func(move |b| {
        if let (Some(s), Some(sc)) = (ws.upgrade(), wsc.upgrade()) {
            fill_gallery(&s, &sc, b);
        }
    });
    let keys = gtk::EventControllerKey::new();
    let ws = Rc::downgrade(s);
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape
            && let Some(s) = ws.upgrade()
        {
            set_editing(&s, false);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);

    window.present();
    // The surface's size settles after it's mapped; the input region follows it.
    if let Some(surface) = window.surface() {
        let (ws, wsc) = (Rc::downgrade(s), Rc::downgrade(&sc));
        surface.connect_layout(move |_, w, h| {
            if let (Some(s), Some(sc)) = (ws.upgrade(), wsc.upgrade())
                && sc.size.replace((w, h)) != (w, h)
            {
                schedule_input(&s, &sc);
            }
        });
    }
    set_input(s, &sc);
    if s.editing.get() {
        apply_editing(s, &sc);
    }
    sc
}

pub fn relayout_all(s: &Rc<Shared>) {
    for sc in s.screens.borrow().clone() {
        relayout(s, &sc);
    }
    host::send_widgets(s);
    let ws = Rc::downgrade(s);
    glib::idle_add_local_once(move || {
        if let Some(s) = ws.upgrade() {
            update_visibility(&s);
        }
    });
}

fn relayout(s: &Rc<Shared>, sc: &Rc<Screen>) {
    let placements = s.layout.borrow().for_monitor(&sc.stable_id, &sc.connector);
    let editing = s.editing.get();
    let mut keep = vec![];
    for (scope, p) in placements {
        let (plugin, part) = split_part(&p.part);
        let (plugin, part) = (plugin.to_string(), part.to_string());
        let (available, size_name, size, interactive, free) = {
            let plugins = s.plugins.borrow();
            match plugins.get(&plugin).filter(|r| r.parts.contains(&part)) {
                Some(r) => {
                    let (name, px) = host::resolve_size(&r.plugin.manifest, &part, &p);
                    let found = r.plugin.manifest.parts(Kind::Desktop).into_iter().find(|x| x.id == part);
                    let interactive = found.as_ref().is_some_and(|x| x.interactive());
                    let free = found.filter(|x| x.resizable()).map(|x| x.free_range());
                    (true, name, px, interactive, free)
                }
                None => {
                    let name = p.size.clone().unwrap_or_else(|| "medium".into());
                    let px = match (p.width, p.height) {
                        (Some(w), Some(h)) if name == "free" => (w, h),
                        _ => nyx_plugin::manifest::desktop_size(&name).unwrap_or((336, 160)),
                    };
                    (false, name, px, false, None)
                }
            }
        };
        let workspace = scope.workspace().map(String::from);
        let existing = sc.tiles.borrow().iter().find(|t| t.plugin == plugin && t.part == part && t.workspace == workspace).cloned();
        let t = existing.unwrap_or_else(|| new_tile(s, sc, &plugin, &part, workspace.clone()));
        if workspace.is_some() {
            t.ws_button.add_css_class("on");
        } else {
            t.ws_button.remove_css_class("on");
        }
        *t.scope.borrow_mut() = scope;
        t.available.set(available);
        t.interactive.set(interactive);
        t.free.set(free);
        t.grip.set_visible(editing && free.is_some());
        t.size_button.set_label(&size_label(&size_name));
        *t.size_name.borrow_mut() = size_name;
        t.size.set(size);
        set_clip_size(&t, size);
        t.card.remove_css_class("bare");
        if p.background == Background::None {
            t.card.add_css_class("bare");
        }
        t.bg_button.set_tooltip_text(Some(if p.background == Background::None {
            "No background: click for a card"
        } else {
            "On a card: click for no background"
        }));
        if p.background == Background::None {
            t.bg_button.remove_css_class("on");
        } else {
            t.bg_button.add_css_class("on");
        }
        *t.placement.borrow_mut() = p;
        apply_position(&t);
        refresh_tile(s, sc, &t);
        t.root.set_visible(t.shown(editing));
        keep.push(t);
    }
    for t in sc.tiles.borrow().iter() {
        if !keep.iter().any(|k| Rc::ptr_eq(k, t)) {
            if let Some((p, _)) = t.popover.borrow_mut().take() {
                p.unparent();
            }
            if let Some(m) = t.menu.borrow_mut().take() {
                m.unparent();
            }
            sc.overlay.remove_overlay(&t.root);
        }
    }
    *sc.tiles.borrow_mut() = keep;
    apply_workspace(s, sc);
    schedule_input(s, sc);
}

/// The monitor's active workspace: its id and its name.
pub fn active_workspace(s: &Shared, sc: &Screen) -> Vec<String> {
    let desktop = s.desktop.borrow();
    let Some(m) = desktop.monitor(&sc.connector) else { return vec![] };
    let mut keys = vec![m.active_workspace.to_string()];
    if let Some(w) = desktop.workspaces.iter().find(|w| w.id == m.active_workspace) {
        keys.push(w.name.clone());
    }
    keys
}

/// Show the widgets of the monitor's active workspace (and those for every workspace
/// it doesn't replace), hide the rest. Nothing is rebuilt: a switch only maps and
/// unmaps.
fn apply_workspace(s: &Shared, sc: &Screen) {
    let keys = active_workspace(s, sc);
    let active: Vec<&str> = keys.iter().map(String::as_str).collect();
    let tiles = sc.tiles.borrow();
    let placements: Vec<(Scope, Placement)> = tiles.iter().map(|t| (t.scope.borrow().clone(), t.placement.borrow().clone())).collect();
    let shown = layout::shown_on(&placements, &active);
    let editing = s.editing.get();
    for (i, t) in tiles.iter().enumerate() {
        t.on_workspace.set(shown.contains(&i));
        t.root.set_visible(t.shown(editing));
        if let Some(w) = active.last() {
            t.ws_button.set_tooltip_text(Some(&if t.workspace.is_some() {
                "Only on this workspace: click to show it on every workspace".to_string()
            } else {
                format!("On every workspace: click to keep it to workspace {w}")
            }));
        }
    }
    drop(tiles);
    update_mapped(s, sc);
}

/// The active workspace changed on some monitor.
pub fn workspaces_changed(s: &Rc<Shared>) {
    for sc in s.screens.borrow().clone() {
        apply_workspace(s, &sc);
        schedule_input(s, &sc);
    }
    host::send_widgets(s);
    update_visibility(s);
}

/// A monitor showing nothing has no surface at all: an empty one costs a redraw for
/// every configure the compositor sends it.
fn update_mapped(s: &Shared, sc: &Screen) {
    let editing = s.editing.get();
    let any = editing || sc.tiles.borrow().iter().any(|t| t.shown(editing));
    if sc.window.is_visible() != any {
        host::debug(|| format!("{} {}", if any { "showing widgets on" } else { "nothing to show on" }, sc.connector));
        sc.window.set_visible(any);
        sc.input.borrow_mut().take();
    }
}

/// The card's padding in style.css: sizes are the card's outside.
const PADDING: i32 = 16;

fn set_clip_size(t: &Tile, size: (i32, i32)) {
    let (w, h) = ((size.0 - 2 * PADDING).max(1), (size.1 - 2 * PADDING).max(1));
    // Shrinking: lower the maximum first, so the minimum never exceeds it.
    t.clip.set_min_content_width(0);
    t.clip.set_min_content_height(0);
    t.clip.set_max_content_width(w);
    t.clip.set_max_content_height(h);
    t.clip.set_min_content_width(w);
    t.clip.set_min_content_height(h);
}

/// A free size while resizing: the drag added to the size it started at, within the
/// part's range, on the grid.
fn resized(t: &Tile, dx: f64, dy: f64) -> (i32, i32) {
    let ((min_w, min_h), (max_w, max_h)) = t.free.get().unwrap_or(((160, 120), (672, 504)));
    let start = t.resize_start.get();
    let w = geometry::snap(start.0 + dx.round() as i32).clamp(min_w, max_w);
    let h = geometry::snap(start.1 + dy.round() as i32).clamp(min_h, max_h);
    (w, h)
}

fn size_label(name: &str) -> String {
    let mut c = name.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// Anchor a tile with alignments and margins, so it stays put when the area changes.
fn apply_position(t: &Tile) {
    let p = t.placement.borrow();
    let (sx, sy) = p.anchor.sides();
    let set = |side: Side, off: i32, align: &dyn Fn(gtk::Align), start: &dyn Fn(i32), end: &dyn Fn(i32)| {
        start(0);
        end(0);
        match side {
            Side::Start => {
                align(gtk::Align::Start);
                start(off.max(0));
            }
            Side::End => {
                align(gtk::Align::End);
                end(off.max(0));
            }
            // Centred with a margin on one side moves it by half the margin.
            Side::Middle => {
                align(gtk::Align::Center);
                if off > 0 { start(off * 2) } else { end(-off * 2) }
            }
        }
    };
    let r = &t.root;
    set(sx, p.offset[0], &|a| r.set_halign(a), &|m| r.set_margin_start(m), &|m| r.set_margin_end(m));
    set(sy, p.offset[1], &|a| r.set_valign(a), &|m| r.set_margin_top(m), &|m| r.set_margin_bottom(m));
}

fn new_tile(s: &Rc<Shared>, sc: &Rc<Screen>, plugin: &str, part: &str, workspace: Option<String>) -> Rc<Tile> {
    let root = gtk::Overlay::new();
    root.add_css_class("dw-tile");
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("nyx-widget");
    card.add_css_class(&view::css::plugin_class(plugin));
    let safe: String = part.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    card.add_css_class(&format!("part-{safe}"));
    card.set_overflow(gtk::Overflow::Hidden);
    let clip = gtk::ScrolledWindow::new();
    clip.set_policy(gtk::PolicyType::External, gtk::PolicyType::External);
    clip.set_propagate_natural_width(false);
    clip.set_propagate_natural_height(false);
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
    wheel.connect_scroll(|_, _, _| glib::Propagation::Stop);
    clip.add_controller(wheel);
    card.append(&clip);
    root.set_child(Some(&card));

    let shield = gtk::Box::new(gtk::Orientation::Vertical, 0);
    shield.add_css_class("dw-shield");
    shield.set_cursor_from_name(Some("grab"));
    shield.set_visible(false);
    root.add_overlay(&shield);

    let chrome = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    chrome.add_css_class("dw-chrome");
    chrome.set_halign(gtk::Align::End);
    chrome.set_valign(gtk::Align::Start);
    chrome.set_margin_top(6);
    chrome.set_margin_end(6);
    chrome.set_visible(false);
    let size_button = gtk::Button::with_label("Medium");
    size_button.set_tooltip_text(Some("Size"));
    let bg_button = gtk::Button::from_icon_name("image-x-generic-symbolic");
    bg_button.set_tooltip_text(Some("Background"));
    let ws_button = gtk::Button::from_icon_name("view-pin-symbolic");
    let remove = gtk::Button::from_icon_name("window-close-symbolic");
    remove.set_tooltip_text(Some("Remove"));
    remove.add_css_class("dw-remove");
    chrome.append(&size_button);
    chrome.append(&bg_button);
    chrome.append(&ws_button);
    chrome.append(&remove);
    root.add_overlay(&chrome);

    let grip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    grip.add_css_class("dw-grip");
    grip.set_halign(gtk::Align::End);
    grip.set_valign(gtk::Align::End);
    grip.set_size_request(22, 22);
    grip.set_cursor_from_name(Some("se-resize"));
    grip.set_tooltip_text(Some("Drag to resize"));
    grip.set_visible(false);
    root.add_overlay(&grip);

    let t = Rc::new(Tile {
        plugin: plugin.into(),
        part: part.into(),
        scope: RefCell::new(Scope::All),
        placement: RefCell::new(Placement::new(plugin)),
        size_name: RefCell::new("medium".into()),
        size: Cell::new((336, 160)),
        interactive: Cell::new(false),
        available: Cell::new(false),
        has_item: Cell::new(false),
        seen: Cell::new(true),
        workspace,
        on_workspace: Cell::new(true),
        root: root.clone(),
        card,
        clip,
        shield: shield.clone(),
        chrome,
        size_button: size_button.clone(),
        bg_button: bg_button.clone(),
        ws_button: ws_button.clone(),
        grip: grip.clone(),
        free: Cell::new(None),
        resize_start: Cell::new((0, 0)),
        popover: RefCell::default(),
        menu: RefCell::default(),
        rendered: RefCell::default(),
        last: RefCell::default(),
        stand_in: Cell::new(StandIn::None),
        drag_start: Cell::new((0, 0)),
    });

    // Clicks reach the plugin only on interactive widgets (elsewhere the input region
    // lets them through), as commands or events.
    let click = gtk::GestureClick::new();
    click.set_button(0);
    let (ws, wsc, wt) = (Rc::downgrade(s), Rc::downgrade(sc), Rc::downgrade(&t));
    click.connect_released(move |g, _, _, _| {
        let (Some(s), Some(sc), Some(t)) = (ws.upgrade(), wsc.upgrade(), wt.upgrade()) else { return };
        on_click(&s, &sc, &t, g.current_button());
    });
    t.card.add_controller(click);

    let drag = gtk::GestureDrag::new();
    let (wsc, wt) = (Rc::downgrade(sc), Rc::downgrade(&t));
    drag.connect_drag_begin(move |_, _, _| {
        let (Some(sc), Some(t)) = (wsc.upgrade(), wt.upgrade()) else { return };
        let b = t.root.compute_bounds(&sc.overlay).map(|b| (b.x() as i32, b.y() as i32)).unwrap_or_default();
        t.drag_start.set(b);
        t.shield.set_cursor_from_name(Some("grabbing"));
        t.root.add_css_class("dragging");
    });
    let (wsc, wt) = (Rc::downgrade(sc), Rc::downgrade(&t));
    drag.connect_drag_update(move |_, dx, dy| {
        let (Some(sc), Some(t)) = (wsc.upgrade(), wt.upgrade()) else { return };
        let (x, y) = drag_pos(&sc, &t, dx, dy);
        let r = &t.root;
        r.set_halign(gtk::Align::Start);
        r.set_valign(gtk::Align::Start);
        r.set_margin_end(0);
        r.set_margin_bottom(0);
        r.set_margin_start(x);
        r.set_margin_top(y);
    });
    let (ws, wsc, wt) = (Rc::downgrade(s), Rc::downgrade(sc), Rc::downgrade(&t));
    drag.connect_drag_end(move |_, dx, dy| {
        let (Some(s), Some(sc), Some(t)) = (ws.upgrade(), wsc.upgrade(), wt.upgrade()) else { return };
        t.shield.set_cursor_from_name(Some("grab"));
        t.root.remove_css_class("dragging");
        let pos = drag_pos(&sc, &t, dx, dy);
        let (anchor, offset) = geometry::locate(area(&sc), tile_size(&t), pos);
        if !write_position(&s, &t, anchor, [offset.0, offset.1]) {
            apply_position(&t);
        }
    });
    shield.add_controller(drag);

    let resize = gtk::GestureDrag::new();
    let wt = Rc::downgrade(&t);
    resize.connect_drag_begin(move |g, _, _| {
        let Some(t) = wt.upgrade() else { return };
        g.set_state(gtk::EventSequenceState::Claimed);
        t.resize_start.set(t.size.get());
        t.root.add_css_class("dragging");
    });
    let wt = Rc::downgrade(&t);
    resize.connect_drag_update(move |_, dx, dy| {
        let Some(t) = wt.upgrade() else { return };
        let (w, h) = resized(&t, dx, dy);
        set_clip_size(&t, (w, h));
    });
    let (ws, wt) = (Rc::downgrade(s), Rc::downgrade(&t));
    resize.connect_drag_end(move |_, dx, dy| {
        let (Some(s), Some(t)) = (ws.upgrade(), wt.upgrade()) else { return };
        t.root.remove_css_class("dragging");
        let (w, h) = resized(&t, dx, dy);
        let (scope, key) = (t.scope.borrow().clone(), t.key());
        if !host::edit_layout(&s, |doc| layout::set_free_size(doc, &scope, &key, w, h)) {
            set_clip_size(&t, t.size.get());
        }
    });
    grip.add_controller(resize);

    let (ws, wt) = (Rc::downgrade(s), Rc::downgrade(&t));
    size_button.connect_clicked(move |_| {
        let (Some(s), Some(t)) = (ws.upgrade(), wt.upgrade()) else { return };
        let sizes: Vec<String> = part_sizes(&s, &t.plugin, &t.part).into_iter().filter(|x| x != "free").collect();
        let current = t.size_name.borrow().clone();
        let next = sizes.iter().position(|x| *x == current).map(|i| sizes[(i + 1) % sizes.len()].clone());
        if let Some(next) = next.or_else(|| sizes.first().cloned()) {
            let scope = t.scope.borrow().clone();
            let key = t.key();
            host::edit_layout(&s, |doc| layout::set_size(doc, &scope, &key, &next));
        }
    });
    let (ws, wt) = (Rc::downgrade(s), Rc::downgrade(&t));
    bg_button.connect_clicked(move |_| {
        let (Some(s), Some(t)) = (ws.upgrade(), wt.upgrade()) else { return };
        let next = if t.placement.borrow().background == Background::Card { Background::None } else { Background::Card };
        let scope = t.scope.borrow().clone();
        let key = t.key();
        host::edit_layout(&s, |doc| layout::set_background(doc, &scope, &key, next));
    });
    let (ws, wsc, wt) = (Rc::downgrade(s), Rc::downgrade(sc), Rc::downgrade(&t));
    ws_button.connect_clicked(move |_| {
        let (Some(s), Some(sc), Some(t)) = (ws.upgrade(), wsc.upgrade(), wt.upgrade()) else { return };
        let from = t.scope.borrow().clone();
        // Keep it to this workspace by its name (which is its id unless it was named).
        let keys = active_workspace(&s, &sc);
        let to = if from.workspace().is_some() { from.with_workspace(None) } else { from.with_workspace(keys.last().map(String::as_str)) };
        let key = t.key();
        host::edit_layout(&s, |doc| layout::move_scope(doc, &from, &to, &key));
    });
    let (ws, wt) = (Rc::downgrade(s), Rc::downgrade(&t));
    remove.connect_clicked(move |_| {
        let (Some(s), Some(t)) = (ws.upgrade(), wt.upgrade()) else { return };
        let scope = t.scope.borrow().clone();
        let key = t.key();
        host::edit_layout(&s, |doc| layout::remove(doc, &scope, &key));
    });

    sc.overlay.add_overlay(&root);
    if s.editing.get() {
        set_tile_editing(&t, true);
    }
    t
}

fn area(sc: &Screen) -> (i32, i32) {
    (sc.overlay.width(), sc.overlay.height())
}

fn tile_size(t: &Tile) -> (i32, i32) {
    let (w, h) = (t.root.width(), t.root.height());
    if w > 0 && h > 0 { (w, h) } else { t.size.get() }
}

fn drag_pos(sc: &Screen, t: &Tile, dx: f64, dy: f64) -> (i32, i32) {
    let (a, size, start) = (area(sc), tile_size(t), t.drag_start.get());
    let x = (start.0 + dx.round() as i32).min(a.0 - size.0).max(0);
    let y = (start.1 + dy.round() as i32).min(a.1 - size.1).max(0);
    (x, y)
}

fn write_position(s: &Rc<Shared>, t: &Tile, anchor: Anchor, offset: [i32; 2]) -> bool {
    let scope = t.scope.borrow().clone();
    let key = t.key();
    host::edit_layout(s, |doc| layout::set_position(doc, &scope, &key, anchor, offset))
}

fn part_sizes(s: &Shared, plugin: &str, part: &str) -> Vec<String> {
    let from = |m: &nyx_plugin::Manifest| m.parts(Kind::Desktop).into_iter().find(|x| x.id == part).map(|x| x.sizes());
    if let Some(sizes) = s.plugins.borrow().get(plugin).and_then(|r| from(&r.plugin.manifest)) {
        return sizes;
    }
    discovery::discover().get(plugin).and_then(|p| from(&p.manifest)).unwrap_or_else(|| vec!["medium".into()])
}

fn on_click(s: &Rc<Shared>, sc: &Rc<Screen>, t: &Rc<Tile>, button: u32) {
    if s.editing.get() {
        return;
    }
    let (cmd, item_id, has_popover, menu) = {
        let plugins = s.plugins.borrow();
        let Some(r) = plugins.get(&t.plugin) else { return };
        let Some(i) = host::item_for(r, &t.part, &sc.connector) else { return };
        let cmd = match button {
            1 => i.on_click.clone(),
            2 => i.on_middle_click.clone(),
            3 => i.on_right_click.clone(),
            _ => None,
        };
        let key = |m: Option<String>| (i.id.clone(), m);
        let has_popover = r.popovers.contains_key(&key(Some(sc.connector.clone()))) || r.popovers.contains_key(&key(None));
        let menu = r.menus.get(&key(Some(sc.connector.clone()))).or_else(|| r.menus.get(&key(None))).cloned();
        (cmd, i.id.clone(), has_popover, menu)
    };
    if let Some(c) = cmd {
        host::run_for(s, &t.plugin, &c);
        return;
    }
    if button == 1 && has_popover {
        open_popover(s, sc, t, &item_id);
        return;
    }
    if button == 3
        && let Some(entries) = menu
    {
        open_menu(s, sc, t, &item_id, &entries);
        return;
    }
    host::send_event(s, &t.plugin, &t.part, &sc.connector, EventKind::Click, Some(button), None, None);
}

/// Popovers and menus take the keyboard while open (a test copy's never do).
fn keyboard_while_open(s: &Shared, sc: &Screen, open: bool) {
    if !s.test() && !s.editing.get() {
        sc.window.set_keyboard_mode(if open { KeyboardMode::OnDemand } else { KeyboardMode::None });
    }
}

fn fill_popover(s: &Rc<Shared>, sc: &Rc<Screen>, t: &Rc<Tile>, item: &str) {
    let node = {
        let plugins = s.plugins.borrow();
        let Some(r) = plugins.get(&t.plugin) else { return };
        r.popovers.get(&(item.to_string(), Some(sc.connector.clone()))).or_else(|| r.popovers.get(&(item.to_string(), None))).cloned()
    };
    let (Some(node), Some(ctx)) = (node, tile_ctx(s, &t.plugin, item, &sc.connector)) else { return };
    let rendered = Rc::new(view::render(&node, &ctx));
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroll.set_propagate_natural_height(true);
    scroll.set_propagate_natural_width(true);
    scroll.set_max_content_height(640);
    scroll.set_max_content_width(480);
    scroll.set_child(Some(&rendered.root));
    let existing = t.popover.borrow().as_ref().map(|(p, _)| p.clone());
    let pop = existing.unwrap_or_else(|| {
        let p = gtk::Popover::new();
        p.add_css_class("dw-popover");
        p.set_has_arrow(false);
        p.set_parent(&t.card);
        if s.test() {
            p.set_autohide(false);
        }
        let (ws, wsc, pid, iid, conn) = (Rc::downgrade(s), Rc::downgrade(sc), t.plugin.clone(), t.part.clone(), sc.connector.clone());
        p.connect_closed(move |_| {
            let (Some(s), Some(sc)) = (ws.upgrade(), wsc.upgrade()) else { return };
            keyboard_while_open(&s, &sc, false);
            host::send_event(&s, &pid, &iid, &conn, EventKind::PopoverClose, None, None, None);
        });
        p
    });
    pop.set_child(Some(&scroll));
    *t.popover.borrow_mut() = Some((pop, rendered));
}

fn open_popover(s: &Rc<Shared>, sc: &Rc<Screen>, t: &Rc<Tile>, item: &str) {
    fill_popover(s, sc, t, item);
    let Some((pop, _)) = t.popover.borrow().clone() else { return };
    keyboard_while_open(s, sc, true);
    pop.popup();
    host::send_event(s, &t.plugin, &t.part, &sc.connector, EventKind::PopoverOpen, Some(1), None, None);
}

/// A plugin changed a popover that's open: draw it again.
pub fn refresh_popover(s: &Rc<Shared>, plugin: &str, item: &str) {
    for sc in s.screens.borrow().clone() {
        for t in sc.tiles.borrow().clone().iter().filter(|t| t.plugin == plugin) {
            let open = t.popover.borrow().as_ref().is_some_and(|(p, _)| p.is_visible());
            if open {
                fill_popover(s, &sc, t, item);
            }
        }
    }
}

fn open_menu(s: &Rc<Shared>, sc: &Rc<Screen>, t: &Rc<Tile>, item: &str, entries: &[MenuEntry]) {
    if let Some(old) = t.menu.borrow_mut().take() {
        old.unparent();
    }
    let (ws, pid, iid, conn) = (Rc::downgrade(s), t.plugin.clone(), item.to_string(), sc.connector.clone());
    let (model, group) = view::menu(
        entries,
        Rc::new(move |id: String| {
            if let Some(s) = ws.upgrade() {
                host::send_event(&s, &pid, &iid, &conn, EventKind::Action, None, Some(id), None);
            }
        }),
    );
    t.card.insert_action_group("pm", Some(&group));
    let menu = gtk::PopoverMenu::from_model(Some(&model));
    menu.add_css_class("dw-popover");
    menu.set_has_arrow(false);
    menu.set_parent(&t.card);
    if s.test() {
        menu.set_autohide(false);
    }
    let (ws, wsc) = (Rc::downgrade(s), Rc::downgrade(sc));
    menu.connect_closed(move |_| {
        if let (Some(s), Some(sc)) = (ws.upgrade(), wsc.upgrade()) {
            keyboard_while_open(&s, &sc, false);
        }
    });
    keyboard_while_open(s, sc, true);
    menu.popup();
    *t.menu.borrow_mut() = Some(menu);
}

// ---------- Drawing what plugins send ----------

fn tile_ctx(s: &Rc<Shared>, plugin: &str, item: &str, connector: &str) -> Option<view::Ctx> {
    let plugins = s.plugins.borrow();
    let r = plugins.get(plugin)?;
    let (ws, pid, iid, conn) = (Rc::downgrade(s), plugin.to_string(), item.to_string(), connector.to_string());
    Some(view::Ctx {
        plugin: plugin.to_string(),
        plugin_dir: r.plugin.dir.clone(),
        storage_dir: nyx_plugin::paths::storage_dir(plugin),
        trusted: r.plugin.trust.is_trusted(),
        max_height: None,
        on_event: Rc::new(move |e: ViewEvent| {
            if let Some(s) = ws.upgrade() {
                host::send_event(&s, &pid, &iid, &conn, e.kind, None, e.control, e.value);
            }
        }),
    })
}

fn refresh_tile(s: &Rc<Shared>, sc: &Screen, t: &Rc<Tile>) {
    if !t.available.get() {
        t.has_item.set(false);
        if t.stand_in.replace(StandIn::Unavailable) != StandIn::Unavailable {
            host::debug(|| format!("{} isn't available", t.key()));
            t.last.borrow_mut().take();
            show_placeholder(s, t);
        }
        return;
    }
    let current = s.plugins.borrow().get(&t.plugin).and_then(|r| host::item_for(r, &t.part, &sc.connector));
    let Some(current) = current.filter(|c| c.visible) else {
        // Shown only in edit mode: what goes here.
        t.has_item.set(false);
        if t.stand_in.replace(StandIn::Empty) != StandIn::Empty {
            host::debug(|| format!("{} has nothing to show", t.key()));
            t.last.borrow_mut().take();
            t.rendered.borrow_mut().take();
            show_note(s, t);
        }
        return;
    };
    t.has_item.set(true);
    t.stand_in.set(StandIn::None);
    t.card.set_tooltip_text(current.tooltip.as_deref());
    if t.last.borrow().as_ref() == Some(&current) {
        return;
    }
    let Some(ctx) = tile_ctx(s, &t.plugin, &current.id, &sc.connector) else { return };
    let node = current.view.clone().unwrap_or_else(|| view::simple(&current));
    let rendered = Rc::new(view::render(&node, &ctx));
    rendered.attach_states(t.card.upcast_ref(), &ctx);
    rendered.root.set_valign(gtk::Align::Center);
    t.clip.set_child(Some(&rendered.root));
    for c in t.card.css_classes() {
        if c.starts_with("item-class-") {
            t.card.remove_css_class(&c);
        }
    }
    for c in &current.class {
        if c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
            t.card.add_css_class(&format!("item-class-{c}"));
        }
    }
    *t.rendered.borrow_mut() = Some(rendered);
    *t.last.borrow_mut() = Some(current);
}

fn show_placeholder(s: &Shared, t: &Tile) {
    t.rendered.borrow_mut().take();
    let name = discovery::discover().get(&t.plugin).map(|p| p.manifest.name.clone()).unwrap_or_else(|| t.key());
    let why = if s.plugins.borrow().contains_key(&t.plugin) { "this part is off" } else { "it isn't installed or turned on" };
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    b.add_css_class("dw-placeholder");
    b.set_valign(gtk::Align::Center);
    b.set_vexpand(true);
    let title = gtk::Label::new(Some(&format!("{name} isn't available")));
    title.add_css_class("title");
    title.set_wrap(true);
    let sub = gtk::Label::new(Some(why));
    sub.add_css_class("dim");
    b.append(&title);
    b.append(&sub);
    t.clip.set_child(Some(&b));
}

fn show_note(s: &Shared, t: &Tile) {
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
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    b.add_css_class("dw-placeholder");
    b.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some(&name));
    title.add_css_class("title");
    let sub = gtk::Label::new(Some("Nothing to show right now"));
    sub.add_css_class("dim");
    b.append(&title);
    b.append(&sub);
    t.clip.set_child(Some(&b));
}

pub fn refresh_plugin(s: &Rc<Shared>, plugin: &str) {
    refresh_previews(s, plugin);
    let editing = s.editing.get();
    for sc in s.screens.borrow().clone() {
        for t in sc.tiles.borrow().iter().filter(|t| t.plugin == plugin) {
            refresh_tile(s, &sc, t);
            t.root.set_visible(t.shown(editing));
        }
        update_mapped(s, &sc);
        schedule_input(s, &sc);
    }
    let ws = Rc::downgrade(s);
    glib::idle_add_local_once(move || {
        if let Some(s) = ws.upgrade() {
            update_visibility(&s);
        }
    });
}

pub fn rerender_all(s: &Rc<Shared>) {
    for sc in s.screens.borrow().clone() {
        for t in sc.tiles.borrow().iter() {
            t.last.borrow_mut().take();
            refresh_tile(s, &sc, t);
        }
    }
}

pub fn patch(s: &Rc<Shared>, plugin: &str, p: &Patch) {
    let part = s.plugins.borrow().get(plugin).and_then(|r| r.plugin.manifest.part_of(Kind::Desktop, &p.item));
    let Some(part) = part else { return };
    for sc in s.screens.borrow().clone() {
        if p.monitor.as_ref().is_some_and(|m| *m != sc.connector) {
            continue;
        }
        for t in sc.tiles.borrow().iter().filter(|t| t.plugin == plugin && t.part == part) {
            let showing = t.last.borrow().as_ref().is_some_and(|i| i.id == p.item);
            if !showing {
                continue;
            }
            if let (Some(r), Some(ctx)) = (t.rendered.borrow().as_ref(), tile_ctx(s, plugin, &p.item, &sc.connector)) {
                for n in &p.nodes {
                    r.patch(n, &ctx);
                }
            }
            // The stored item already has the patch; don't draw it again for it.
            let latest = s.plugins.borrow().get(plugin).and_then(|r| host::item_for(r, &part, &sc.connector));
            *t.last.borrow_mut() = latest;
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn show_frame(s: &Shared, plugin: &str, item: &str, node: &str, bytes: &[u8], width: i32, height: i32, stride: i32) {
    for sc in s.screens.borrow().iter() {
        for t in sc.tiles.borrow().iter().filter(|t| t.plugin == plugin && t.seen.get()) {
            if !t.last.borrow().as_ref().is_some_and(|i| i.id == item) {
                continue;
            }
            if let Some(r) = t.rendered.borrow().as_ref()
                && let Some(pic) = r.frame(node)
            {
                view::show_frame(&pic, bytes, width, height, stride);
            }
        }
    }
}

// ---------- Covered by windows ----------

/// Which widgets windows cover, per monitor: the windows on its shown workspaces, or
/// everything when one is fullscreen. Plugins hear which of theirs can be seen.
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
        let origin = (m.x + m.reserved[0], m.y + m.reserved[1]);
        for t in sc.tiles.borrow().iter() {
            let rect = t.root.compute_bounds(&sc.window).map(|b| {
                Rect::new(b.x() as i32, b.y() as i32, b.width() as i32, b.height() as i32).translate(origin.0, origin.1)
            });
            let covered = locked || fullscreen || rect.is_some_and(|r| geometry::covered(&r, &windows));
            let v = t.shown(editing) && !covered;
            t.seen.set(v);
            let e = seen.entry((t.plugin.clone(), t.part.clone())).or_insert(false);
            *e |= v;
        }
    }
    host::apply_visibility(s, &seen);
}

// ---------- Input ----------

fn schedule_input(s: &Rc<Shared>, sc: &Rc<Screen>) {
    let (ws, wsc) = (Rc::downgrade(s), Rc::downgrade(sc));
    glib::idle_add_local_once(move || {
        if let (Some(s), Some(sc)) = (ws.upgrade(), wsc.upgrade()) {
            set_input(&s, &sc);
        }
    });
}

/// Outside edit mode only interactive widgets take clicks; the rest of the surface lets
/// them through to whatever is below.
fn set_input(s: &Shared, sc: &Screen) {
    let Some(surface) = sc.window.surface() else { return };
    let region = gtk::cairo::Region::create();
    let mut rects = vec![];
    if s.editing.get() {
        rects.push(gtk::cairo::RectangleInt::new(0, 0, sc.window.width().max(1), sc.window.height().max(1)));
    } else {
        for t in sc.tiles.borrow().iter().filter(|t| t.interactive.get() && t.root.is_visible()) {
            if let Some(b) = t.card.compute_bounds(&sc.window) {
                rects.push(gtk::cairo::RectangleInt::new(b.x() as i32, b.y() as i32, b.width() as i32, b.height() as i32));
            }
        }
    }
    let key: Vec<Region> = rects.iter().map(|r| (r.x(), r.y(), r.width(), r.height())).collect();
    if sc.input.borrow().as_ref() == Some(&key) {
        return;
    }
    *sc.input.borrow_mut() = Some(key);
    for r in &rects {
        let _ = region.union_rectangle(r);
    }
    host::debug(|| {
        let list: Vec<String> = rects.iter().map(|r| format!("{}x{}+{}+{}", r.width(), r.height(), r.x(), r.y())).collect();
        format!("input region on {}: [{}]", sc.connector, list.join(", "))
    });
    surface.set_input_region(Some(&region));
}

// ---------- Edit mode ----------

pub fn set_editing(s: &Rc<Shared>, on: bool) {
    if s.editing.replace(on) == on {
        return;
    }
    host::debug(|| format!("edit mode {}", if on { "on" } else { "off" }));
    for sc in s.screens.borrow().clone() {
        apply_editing(s, &sc);
    }
    update_visibility(s);
}

fn set_tile_editing(t: &Tile, on: bool) {
    t.shield.set_visible(on);
    t.chrome.set_visible(on);
    t.grip.set_visible(on && t.free.get().is_some());
    if on {
        t.root.add_css_class("editing");
    } else {
        t.root.remove_css_class("editing");
    }
}

fn apply_editing(s: &Rc<Shared>, sc: &Rc<Screen>) {
    let on = s.editing.get();
    if on {
        sc.window.add_css_class("editing");
    } else {
        sc.window.remove_css_class("editing");
    }
    sc.toolbar.set_visible(on);
    // Esc leaves edit mode; a test copy never takes the keyboard.
    sc.window.set_keyboard_mode(if on && !s.test() { KeyboardMode::OnDemand } else { KeyboardMode::None });
    for t in sc.tiles.borrow().iter() {
        set_tile_editing(t, on);
        t.root.set_visible(t.shown(on));
    }
    update_mapped(s, sc);
    set_input(s, sc);
}

/// A widget drawn in the **Add widget** list, from the plugin's `preview` copy.
struct Preview {
    plugin: String,
    part: String,
    clip: gtk::ScrolledWindow,
    last: RefCell<Option<Item>>,
}

thread_local! {
    static PREVIEWS: RefCell<Vec<std::rc::Weak<Preview>>> = RefCell::default();
}

fn refresh_previews(s: &Rc<Shared>, plugin: &str) {
    let previews: Vec<Rc<Preview>> = PREVIEWS.with(|p| {
        let mut p = p.borrow_mut();
        p.retain(|w| w.strong_count() > 0);
        p.iter().filter_map(std::rc::Weak::upgrade).filter(|p| p.plugin == plugin).collect()
    });
    for p in previews {
        let current = s.plugins.borrow().get(&p.plugin).and_then(|r| host::item_for(r, &p.part, host::PREVIEW));
        let Some(current) = current else { continue };
        if p.last.borrow().as_ref() == Some(&current) {
            continue;
        }
        if !current.visible {
            let l = gtk::Label::new(Some("Nothing to show right now"));
            l.add_css_class("dim");
            p.clip.set_child(Some(&l));
            *p.last.borrow_mut() = Some(current);
            continue;
        }
        let Some(ctx) = tile_ctx(s, &p.plugin, &current.id, host::PREVIEW) else { continue };
        let node = current.view.clone().unwrap_or_else(|| view::simple(&current));
        let r = view::render(&node, &ctx);
        r.root.set_valign(gtk::Align::Center);
        p.clip.set_child(Some(&r.root));
        *p.last.borrow_mut() = Some(current);
    }
}

/// The **Add widget** list: every turned-on desktop part not on this monitor yet, drawn
/// live by its plugin (which runs while the list is open), then the installed ones that
/// aren't turned on.
fn fill_gallery(s: &Rc<Shared>, sc: &Rc<Screen>, button: &gtk::MenuButton) {
    let state = State::load();
    let on_screen: Vec<String> = sc.tiles.borrow().iter().map(|t| t.key()).collect();
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_max_children_per_line(2);
    flow.set_min_children_per_line(1);
    flow.set_column_spacing(12);
    flow.set_row_spacing(12);
    flow.set_valign(gtk::Align::Start);
    let mut off = vec![];
    let mut previews = vec![];
    let mut keep = vec![];
    let pop = gtk::Popover::new();
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
            let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
            card.add_css_class("nyx-widget");
            card.add_css_class(&view::css::plugin_class(&p.manifest.id));
            let clip = gtk::ScrolledWindow::new();
            clip.set_policy(gtk::PolicyType::External, gtk::PolicyType::External);
            clip.set_propagate_natural_width(false);
            clip.set_propagate_natural_height(false);
            let (w, h) = ((px.0 - 2 * PADDING).max(1), (px.1 - 2 * PADDING).max(1));
            clip.set_min_content_width(w);
            clip.set_max_content_width(w);
            clip.set_min_content_height(h);
            clip.set_max_content_height(h);
            clip.set_can_target(false);
            let waiting = gtk::Label::new(Some("Starting…"));
            waiting.add_css_class("dim");
            clip.set_child(Some(&waiting));
            card.append(&clip);
            let add = gtk::Button::with_label(&format!("Add {name}"));
            add.add_css_class("dw-gallery-row");
            if !part.description.is_empty() {
                add.set_tooltip_text(Some(&part.description));
            }
            let (ws, wsc, wpop, k) = (Rc::downgrade(s), Rc::downgrade(sc), pop.downgrade(), key.clone());
            add.connect_clicked(move |_| {
                let (Some(s), Some(sc)) = (ws.upgrade(), wsc.upgrade()) else { return };
                add_widget(&s, &sc, &k, &size);
                if let Some(p) = wpop.upgrade() {
                    p.popdown();
                }
            });
            let cell = gtk::Box::new(gtk::Orientation::Vertical, 6);
            cell.append(&card);
            cell.append(&add);
            flow.insert(&cell, -1);
            let preview = Rc::new(Preview { plugin: p.manifest.id.clone(), part: part.id.clone(), clip, last: RefCell::default() });
            PREVIEWS.with(|v| v.borrow_mut().push(Rc::downgrade(&preview)));
            keep.push(preview);
            previews.push((p.manifest.id.clone(), part.id.clone()));
        }
    }
    let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
    list.add_css_class("dw-gallery");
    if previews.is_empty() {
        let l = gtk::Label::new(Some("Every widget that's turned on is already here."));
        l.add_css_class("dim");
        list.append(&l);
    } else {
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroll.set_propagate_natural_width(true);
        scroll.set_propagate_natural_height(true);
        scroll.set_max_content_height(560);
        scroll.set_child(Some(&flow));
        list.append(&scroll);
    }
    off.sort();
    off.dedup();
    if !off.is_empty() {
        let l = gtk::Label::new(Some("Turn on in a terminal:"));
        l.add_css_class("dim");
        l.set_xalign(0.0);
        l.set_margin_top(8);
        list.append(&l);
        for id in off {
            let c = gtk::Label::new(Some(&format!("nyx plugin configure {id}")));
            c.add_css_class("monospace");
            c.set_xalign(0.0);
            c.set_selectable(true);
            list.append(&c);
        }
    }
    pop.add_css_class("dw-popover");
    pop.set_child(Some(&list));
    if s.test() {
        pop.set_autohide(false);
    }
    // The plugins behind the previews run while the list is open.
    *s.previews.borrow_mut() = previews;
    let ws = Rc::downgrade(s);
    let held = RefCell::new(Some(keep));
    pop.connect_closed(move |_| {
        held.borrow_mut().take();
        if let Some(s) = ws.upgrade() {
            s.previews.borrow_mut().clear();
            host::schedule_sync(&s);
        }
    });
    button.set_popover(Some(&pop));
    host::sync_plugins(s);
    let plugins: Vec<String> = s.previews.borrow().iter().map(|(p, _)| p.clone()).collect();
    for p in plugins {
        refresh_previews(s, &p);
    }
}

/// Put a part on this monitor, top left, below what's there already.
fn add_widget(s: &Rc<Shared>, sc: &Screen, key: &str, size: &str) {
    let scope = Scope::Monitor(s.layout.borrow().monitor_key(&sc.stable_id, &sc.connector));
    let below = sc.tiles.borrow().iter().filter(|t| t.placement.borrow().anchor == Anchor::TopLeft).count() as i32;
    let mut p = Placement::new(key);
    p.size = Some(size.to_string());
    p.offset = [48, geometry::snap(48 + below * 184)];
    host::edit_layout(s, |doc| {
        layout::add(doc, &scope, &p);
        true
    });
    host::schedule_sync(s);
}

// ---------- For D-Bus ----------

/// For tests (`TestGallery`, only with `NYX_DW_DEBUG`): edit mode with the **Add
/// widget** list open on the first monitor.
pub fn test_gallery(s: &Rc<Shared>) {
    set_editing(s, true);
    if let Some(sc) = s.screens.borrow().first() {
        sc.add.popup();
    }
}

fn find(s: &Shared, part: &str, monitor: &str) -> Option<(Rc<Screen>, Rc<Tile>)> {
    let sc = s.screens.borrow().iter().find(|sc| sc.connector == monitor || sc.stable_id == monitor).cloned()?;
    let t = sc.tiles.borrow().iter().find(|t| t.key() == part && t.on_workspace.get()).cloned()?;
    Some((sc, t))
}

/// For tests (`TestClick`, only with `NYX_DW_DEBUG`): act as if a widget was clicked.
pub fn test_click(s: &Rc<Shared>, part: &str, monitor: &str, button: u32) -> bool {
    let Some((sc, t)) = find(s, part, monitor) else { return false };
    on_click(s, &sc, &t, button);
    true
}

pub fn move_widget(s: &Rc<Shared>, part: &str, monitor: &str, anchor: &str, offset: [i32; 2]) -> bool {
    let (Some((_, t)), Some(anchor)) = (find(s, part, monitor), Anchor::parse(anchor)) else { return false };
    write_position(s, &t, anchor, [geometry::snap(offset[0]), geometry::snap(offset[1])])
}

pub fn resize_widget(s: &Rc<Shared>, part: &str, monitor: &str, size: &str) -> bool {
    let Some((_, t)) = find(s, part, monitor) else { return false };
    if !part_sizes(s, &t.plugin, &t.part).iter().any(|x| x == size) {
        return false;
    }
    let scope = t.scope.borrow().clone();
    host::edit_layout(s, |doc| layout::set_size(doc, &scope, part, size))
}

pub fn list(s: &Shared) -> serde_json::Value {
    let mut out = vec![];
    for sc in s.screens.borrow().iter() {
        for t in sc.tiles.borrow().iter() {
            let p = t.placement.borrow();
            let bounds = t.root.compute_bounds(&sc.window).map(|b| [b.x() as i32, b.y() as i32, b.width() as i32, b.height() as i32]);
            out.push(serde_json::json!({
                "monitor": sc.connector,
                "stable_id": sc.stable_id,
                "part": t.key(),
                "scope": t.scope.borrow().describe(),
                "on_workspace": t.on_workspace.get(),
                "anchor": p.anchor.name(),
                "offset": p.offset,
                "size": *t.size_name.borrow(),
                "background": p.background.name(),
                "interactive": t.interactive.get(),
                "available": t.available.get(),
                "shown": t.root.is_visible(),
                "visible": t.seen.get(),
                "bounds": bounds,
            }));
        }
    }
    serde_json::Value::Array(out)
}
