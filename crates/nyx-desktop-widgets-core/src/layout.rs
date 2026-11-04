//! `~/.config/nyx/desktop-widgets.toml`:
//!
//! ```toml
//! version = 1
//!
//! [[all.widget]]                       # on every monitor
//! part = "nyx.clock/large"
//! anchor = "top-left"                  # nine anchors; offsets in logical px
//! offset = [48, 48]
//! size = "large"
//!
//! [[monitor."LG Electronics|27GN950|012345".widget]]   # or a connector, "DP-1"
//! part = "nyx.weather/card"
//! anchor = "top-right"
//! background = "none"
//! options = { units = "imperial" }
//!
//! [plugins."nyx.weather"]              # settings, as in bar.toml
//! units = "metric"
//! ```
//!
//! A monitor's own entry for a part replaces the `all` one there. Entries for monitors
//! that aren't plugged in stay until they come back.
//!
//! Widgets can also belong to one workspace, `[[all.workspace."3".widget]]` or
//! `[[monitor."DP-1".workspace."3".widget]]` (a workspace's id or name): they show
//! only while it's the monitor's active one, and on it replace the same part's widget
//! from the lists for every workspace.

use crate::geometry::Anchor;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use toml_edit::{Array, DocumentMut, Item, Table, value};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Background {
    /// The theme's card colour and radius.
    #[default]
    Card,
    /// Nothing: the widget draws straight on the wallpaper.
    None,
}

impl Background {
    pub fn name(self) -> &'static str {
        match self {
            Background::Card => "card",
            Background::None => "none",
        }
    }
}

fn default_offset() -> [i32; 2] {
    [48, 48]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    /// `plugin/part`, or a plugin id for a plugin without parts.
    pub part: String,
    #[serde(default)]
    pub anchor: Anchor,
    #[serde(default = "default_offset")]
    pub offset: [i32; 2],
    /// A size name, or `free` with `width` and `height`; none: the part's default.
    #[serde(default)]
    pub size: Option<String>,
    /// For `size = "free"` (resizable parts), logical pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<i32>,
    #[serde(default)]
    pub background: Background,
    #[serde(default)]
    pub options: toml::Table,
}

impl Placement {
    pub fn new(part: &str) -> Placement {
        Placement {
            part: part.into(),
            anchor: Anchor::TopLeft,
            offset: default_offset(),
            size: None,
            width: None,
            height: None,
            background: Background::Card,
            options: toml::Table::new(),
        }
    }

    /// The plugin id and the part within it (`main` for a whole plugin).
    pub fn split(&self) -> (&str, &str) {
        split_part(&self.part)
    }
}

pub fn split_part(part: &str) -> (&str, &str) {
    part.split_once('/').unwrap_or((part, "main"))
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
struct Group {
    #[serde(default)]
    widget: Vec<Placement>,
    /// Widgets for one workspace, by its id or name.
    #[serde(default)]
    workspace: BTreeMap<String, Group>,
}

/// Which list a placement comes from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    All,
    /// A monitor's stable id or connector, as written in the file.
    Monitor(String),
    /// One workspace, on every monitor.
    AllWorkspace(String),
    /// One workspace on one monitor: (monitor, workspace).
    MonitorWorkspace(String, String),
}

impl Scope {
    /// The workspace a placement belongs to; none: every workspace.
    pub fn workspace(&self) -> Option<&str> {
        match self {
            Scope::AllWorkspace(w) | Scope::MonitorWorkspace(_, w) => Some(w),
            _ => None,
        }
    }

    /// The same list, for one workspace or (`None`) for every one.
    pub fn with_workspace(&self, workspace: Option<&str>) -> Scope {
        let monitor = match self {
            Scope::Monitor(m) | Scope::MonitorWorkspace(m, _) => Some(m.clone()),
            _ => None,
        };
        match (monitor, workspace) {
            (None, None) => Scope::All,
            (Some(m), None) => Scope::Monitor(m),
            (None, Some(w)) => Scope::AllWorkspace(w.into()),
            (Some(m), Some(w)) => Scope::MonitorWorkspace(m, w.into()),
        }
    }

    /// As people would say it, for `ListWidgets`.
    pub fn describe(&self) -> String {
        match self {
            Scope::All => "all".into(),
            Scope::Monitor(m) => m.clone(),
            Scope::AllWorkspace(w) => format!("all, workspace {w}"),
            Scope::MonitorWorkspace(m, w) => format!("{m}, workspace {w}"),
        }
    }
}

/// Which placements show while `active` (a workspace's id and name) is the monitor's
/// workspace: those for every workspace, except where the active one has its own
/// widget of the same part, and the active one's. Indexes into `placements`.
pub fn shown_on(placements: &[(Scope, Placement)], active: &[&str]) -> Vec<usize> {
    let on_active = |s: &Scope| s.workspace().is_some_and(|w| active.contains(&w));
    let overridden: HashSet<&str> = placements.iter().filter(|(s, _)| on_active(s)).map(|(_, p)| p.part.as_str()).collect();
    placements
        .iter()
        .enumerate()
        .filter(|(_, (s, p))| if s.workspace().is_none() { !overridden.contains(p.part.as_str()) } else { on_active(s) })
        .map(|(i, _)| i)
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Layout {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    all: Group,
    #[serde(default)]
    monitor: BTreeMap<String, Group>,
    /// Settings per plugin; `limits` inside is the plugin's limits.
    #[serde(default)]
    pub plugins: BTreeMap<String, toml::Table>,
    /// `gpu` draws with GTK's GPU renderer instead of cairo (more memory).
    #[serde(default)]
    pub renderer: Option<String>,
}

impl Layout {
    /// The file's layout, or an empty one when there's no file; an error when it's broken.
    pub fn load(path: &Path) -> Result<Layout, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Layout::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(_) => Ok(Layout::default()),
        }
    }

    pub fn parse(text: &str) -> Result<Layout, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// The monitor key used in the file for a monitor: its stable id if the file uses
    /// it, else its connector if the file uses that, else the stable id.
    pub fn monitor_key(&self, stable_id: &str, connector: &str) -> String {
        if !self.monitor.contains_key(stable_id) && self.monitor.contains_key(connector) {
            connector.to_string()
        } else {
            stable_id.to_string()
        }
    }

    /// What can show on one monitor: each part once for every workspace, then each part
    /// once per workspace that has its own (see [`shown_on`]).
    pub fn for_monitor(&self, stable_id: &str, connector: &str) -> Vec<(Scope, Placement)> {
        let key = self.monitor_key(stable_id, connector);
        let mon = self.monitor.get(&key);
        let mut out = vec![];
        let mut add = |list: [(Option<&Group>, Scope); 2]| {
            let mut seen = HashSet::new();
            for (g, scope) in list {
                for p in g.map(|g| g.widget.as_slice()).unwrap_or_default() {
                    if seen.insert(p.part.clone()) {
                        out.push((scope.clone(), p.clone()));
                    }
                }
            }
        };
        add([(mon, Scope::Monitor(key.clone())), (Some(&self.all), Scope::All)]);
        let mut workspaces: Vec<&String> = self.all.workspace.keys().chain(mon.map(|m| m.workspace.keys()).into_iter().flatten()).collect();
        workspaces.sort();
        workspaces.dedup();
        for w in workspaces {
            add([
                (mon.and_then(|m| m.workspace.get(w)), Scope::MonitorWorkspace(key.clone(), w.clone())),
                (self.all.workspace.get(w), Scope::AllWorkspace(w.clone())),
            ]);
        }
        out
    }

    /// Every plugin with a widget anywhere, monitors that aren't plugged in included.
    pub fn plugins_placed(&self) -> Vec<String> {
        fn walk<'a>(g: &'a Group, out: &mut Vec<&'a Placement>) {
            out.extend(g.widget.iter());
            for w in g.workspace.values() {
                walk(w, out);
            }
        }
        let mut all = vec![];
        walk(&self.all, &mut all);
        for g in self.monitor.values() {
            walk(g, &mut all);
        }
        let mut out: Vec<String> = all.into_iter().map(|p| p.split().0.to_string()).collect();
        out.sort();
        out.dedup();
        out
    }

    pub fn monitors(&self) -> impl Iterator<Item = &String> {
        self.monitor.keys()
    }

    /// A plugin's settings: its manifest's defaults, then the file's.
    pub fn settings(&self, id: &str, defaults: &toml::Table) -> serde_json::Value {
        let mut t = defaults.clone();
        if let Some(user) = self.plugins.get(id) {
            for (k, v) in user {
                if k != "limits" {
                    t.insert(k.clone(), v.clone());
                }
            }
        }
        serde_json::to_value(t).unwrap_or_default()
    }

    pub fn limits(&self, id: &str) -> Option<&toml::Table> {
        self.plugins.get(id).and_then(|p| p.get("limits")).and_then(|l| l.as_table())
    }
}

// ---------- Editing, keeping the file's comments ----------

fn group_mut<'a>(doc: &'a mut DocumentMut, scope: &Scope, create: bool) -> Option<&'a mut toml_edit::ArrayOfTables> {
    if create && !doc.contains_key("version") {
        doc.insert("version", value(1));
    }
    fn child<'a>(t: &'a mut Table, key: &str, create: bool) -> Option<&'a mut Table> {
        if !t.contains_table(key) {
            if !create {
                return None;
            }
            let mut n = Table::new();
            n.set_implicit(true);
            t.insert(key, Item::Table(n));
        }
        t[key].as_table_mut()
    }
    let base = scope.with_workspace(None);
    let parent: &mut Table = match &base {
        Scope::All => {
            if !doc.contains_table("all") {
                if !create {
                    return None;
                }
                doc.insert("all", Item::Table(Table::new()));
            }
            doc["all"].as_table_mut()?
        }
        Scope::Monitor(key) => {
            if !doc.contains_table("monitor") {
                if !create {
                    return None;
                }
                let mut t = Table::new();
                t.set_implicit(true);
                doc.insert("monitor", Item::Table(t));
            }
            let monitors = doc["monitor"].as_table_mut()?;
            if !monitors.contains_table(key) {
                if !create {
                    return None;
                }
                let mut t = Table::new();
                t.set_implicit(true);
                monitors.insert(key, Item::Table(t));
            }
            monitors[key.as_str()].as_table_mut()?
        }
        _ => unreachable!("with_workspace(None) is All or Monitor"),
    };
    let parent = match scope.workspace() {
        Some(w) => child(child(parent, "workspace", create)?, w, create)?,
        None => parent,
    };
    if !parent.get("widget").is_some_and(|w| w.is_array_of_tables()) {
        if !create {
            return None;
        }
        parent.insert("widget", Item::ArrayOfTables(toml_edit::ArrayOfTables::new()));
    }
    parent.get_mut("widget")?.as_array_of_tables_mut()
}

fn entry_mut<'a>(doc: &'a mut DocumentMut, scope: &Scope, part: &str) -> Option<&'a mut Table> {
    group_mut(doc, scope, false)?.iter_mut().find(|t| t.get("part").and_then(|p| p.as_str()) == Some(part))
}

fn offset_value(offset: [i32; 2]) -> Item {
    let mut a = Array::new();
    a.push(offset[0] as i64);
    a.push(offset[1] as i64);
    value(a)
}

/// Move a widget; false when there's no such entry.
pub fn set_position(doc: &mut DocumentMut, scope: &Scope, part: &str, anchor: Anchor, offset: [i32; 2]) -> bool {
    let Some(t) = entry_mut(doc, scope, part) else { return false };
    t.insert("anchor", value(anchor.name()));
    t.insert("offset", offset_value(offset));
    true
}

pub fn set_size(doc: &mut DocumentMut, scope: &Scope, part: &str, size: &str) -> bool {
    let Some(t) = entry_mut(doc, scope, part) else { return false };
    t.insert("size", value(size));
    t.remove("width");
    t.remove("height");
    true
}

/// Any size, for resizable parts.
pub fn set_free_size(doc: &mut DocumentMut, scope: &Scope, part: &str, width: i32, height: i32) -> bool {
    let Some(t) = entry_mut(doc, scope, part) else { return false };
    t.insert("size", value("free"));
    t.insert("width", value(width as i64));
    t.insert("height", value(height as i64));
    true
}

pub fn set_background(doc: &mut DocumentMut, scope: &Scope, part: &str, background: Background) -> bool {
    let Some(t) = entry_mut(doc, scope, part) else { return false };
    if background == Background::Card {
        t.remove("background");
    } else {
        t.insert("background", value(background.name()));
    }
    true
}

pub fn remove(doc: &mut DocumentMut, scope: &Scope, part: &str) -> bool {
    let Some(g) = group_mut(doc, scope, false) else { return false };
    let before = g.len();
    g.retain(|t| t.get("part").and_then(|p| p.as_str()) != Some(part));
    g.len() != before
}

/// Move a widget to another list (every workspace, or one), keeping all its settings;
/// one already there for the same part is replaced.
pub fn move_scope(doc: &mut DocumentMut, from: &Scope, to: &Scope, part: &str) -> bool {
    if from == to {
        return false;
    }
    let Some(entry) = entry_mut(doc, from, part).map(|t| t.clone()) else { return false };
    remove(doc, from, part);
    remove(doc, to, part);
    group_mut(doc, to, true).expect("created").push(entry);
    true
}

/// Add a widget, or move it if the scope has it already.
pub fn add(doc: &mut DocumentMut, scope: &Scope, p: &Placement) {
    if set_position(doc, scope, &p.part, p.anchor, p.offset) {
        if let Some(size) = &p.size {
            set_size(doc, scope, &p.part, size);
        }
        return;
    }
    let g = group_mut(doc, scope, true).expect("created");
    let mut t = Table::new();
    t.insert("part", value(&p.part));
    t.insert("anchor", value(p.anchor.name()));
    t.insert("offset", offset_value(p.offset));
    if let Some(size) = &p.size {
        t.insert("size", value(size));
    }
    if p.background != Background::Card {
        t.insert("background", value(p.background.name()));
    }
    g.push(t);
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"# My widgets
version = 1

[[all.widget]]
part = "nyx.clock/large"
anchor = "top-left"
offset = [48, 48]
size = "large"

[[all.widget]]
part = "nyx.weather/card"   # the forecast
anchor = "top-right"

[[monitor."LG Electronics|27GN950|012345".widget]]
part = "nyx.weather/card"
anchor = "bottom-right"
offset = [24, 24]
size = "medium"
background = "none"
options = { units = "imperial" }

[[monitor."DP-9".widget]]
part = "acme.ping"

[plugins."nyx.weather"]
units = "metric"
[plugins."nyx.weather".limits]
memory = "64M"
"#;

    #[test]
    fn parses_and_picks_per_monitor() {
        let l = Layout::parse(FILE).unwrap();
        let lg = l.for_monitor("LG Electronics|27GN950|012345", "DP-1");
        assert_eq!(lg.len(), 2);
        let weather = lg.iter().find(|(_, p)| p.part == "nyx.weather/card").unwrap();
        assert_eq!(weather.0, Scope::Monitor("LG Electronics|27GN950|012345".into()));
        assert_eq!(weather.1.anchor, Anchor::BottomRight);
        assert_eq!(weather.1.background, Background::None);
        assert_eq!(weather.1.options["units"].as_str(), Some("imperial"));
        // Anywhere else, the `all` entries with their defaults.
        let other = l.for_monitor("Dell|U2720Q|1", "DP-2");
        let weather = other.iter().find(|(_, p)| p.part == "nyx.weather/card").unwrap();
        assert_eq!(weather.0, Scope::All);
        assert_eq!(weather.1.offset, [48, 48]);
        assert_eq!(weather.1.size, None);
        assert_eq!(weather.1.split(), ("nyx.weather", "card"));
    }

    #[test]
    fn falls_back_to_the_connector() {
        let l = Layout::parse(FILE).unwrap();
        let parts: Vec<String> = l.for_monitor("Acme|X|1", "DP-9").into_iter().map(|(_, p)| p.part).collect();
        assert!(parts.contains(&"acme.ping".to_string()));
        assert_eq!(Placement::new("acme.ping").split(), ("acme.ping", "main"));
        assert_eq!(l.monitor_key("Acme|X|1", "DP-9"), "DP-9");
        assert_eq!(l.monitor_key("Acme|X|1", "DP-3"), "Acme|X|1");
    }

    #[test]
    fn keeps_unplugged_monitors() {
        let l = Layout::parse(FILE).unwrap();
        assert_eq!(l.monitors().count(), 2);
        assert_eq!(l.plugins_placed(), vec!["acme.ping", "nyx.clock", "nyx.weather"]);
    }

    #[test]
    fn settings_and_limits() {
        let l = Layout::parse(FILE).unwrap();
        let defaults: toml::Table = "units = \"metric\"\nhours = 6".parse().unwrap();
        let s = l.settings("nyx.weather", &defaults);
        assert_eq!(s["hours"], 6);
        assert!(s.get("limits").is_none());
        assert_eq!(l.limits("nyx.weather").unwrap()["memory"].as_str(), Some("64M"));
    }

    #[test]
    fn edits_keep_comments() {
        let mut doc: DocumentMut = FILE.parse().unwrap();
        assert!(set_position(&mut doc, &Scope::All, "nyx.weather/card", Anchor::Left, [16, -32]));
        assert!(set_size(&mut doc, &Scope::All, "nyx.clock/large", "compact"));
        assert!(!set_size(&mut doc, &Scope::All, "nope", "compact"));
        let mon = Scope::Monitor("LG Electronics|27GN950|012345".into());
        assert!(set_background(&mut doc, &mon, "nyx.weather/card", Background::Card));
        let text = doc.to_string();
        assert!(text.contains("# My widgets"));
        assert!(text.contains("# the forecast"));
        let l = Layout::parse(&text).unwrap();
        let other = l.for_monitor("x", "y");
        let w = other.iter().find(|(_, p)| p.part == "nyx.weather/card").unwrap();
        assert_eq!((w.1.anchor, w.1.offset), (Anchor::Left, [16, -32]));
        assert_eq!(other[0].1.size.as_deref(), Some("compact"));
        let lg = l.for_monitor("LG Electronics|27GN950|012345", "DP-1");
        assert_eq!(lg.iter().find(|(_, p)| p.part == "nyx.weather/card").unwrap().1.background, Background::Card);
    }

    #[test]
    fn workspaces() {
        let text = r#"
[[all.widget]]
part = "nyx.clock/large"
anchor = "top-left"

[[all.widget]]
part = "nyx.stats/graphs"

[[all.workspace."2".widget]]
part = "nyx.clock/large"
anchor = "center"

[[monitor."DP-1".workspace."2".widget]]
part = "nyx.media/player"

[[all.workspace.code.widget]]
part = "nyx.upnext/card"
"#;
        let l = Layout::parse(text).unwrap();
        let ps = l.for_monitor("DP-1", "DP-1");
        let parts = |idx: Vec<usize>| -> Vec<String> {
            let mut v: Vec<String> = idx.into_iter().map(|i| format!("{}@{}", ps[i].1.part, ps[i].0.workspace().unwrap_or("*"))).collect();
            v.sort();
            v
        };
        // Workspace 1: only the widgets for every workspace.
        assert_eq!(parts(shown_on(&ps, &["1", "1"])), vec!["nyx.clock/large@*", "nyx.stats/graphs@*"]);
        // Workspace 2: its own clock replaces the shared one; its media player joins.
        assert_eq!(parts(shown_on(&ps, &["2", "2"])), vec!["nyx.clock/large@2", "nyx.media/player@2", "nyx.stats/graphs@*"]);
        // A workspace named "code" (id 7).
        assert_eq!(parts(shown_on(&ps, &["7", "code"])), vec!["nyx.clock/large@*", "nyx.stats/graphs@*", "nyx.upnext/card@code"]);
        // Another monitor doesn't get DP-1's media player.
        let other = l.for_monitor("DP-2", "DP-2");
        assert!(!other.iter().any(|(_, p)| p.part == "nyx.media/player"));
        assert!(l.plugins_placed().contains(&"nyx.upnext".to_string()));
    }

    #[test]
    fn moves_between_workspaces() {
        let mut doc: DocumentMut = "# mine\n[[all.widget]]\npart = \"nyx.clock/large\"\nsize = \"large\"\n".parse().unwrap();
        let ws = Scope::All.with_workspace(Some("3"));
        assert_eq!(ws, Scope::AllWorkspace("3".into()));
        assert!(move_scope(&mut doc, &Scope::All, &ws, "nyx.clock/large"));
        let text = doc.to_string();
        assert!(text.contains("# mine") && text.contains("[[all.workspace.3.widget]]"), "{text}");
        let l = Layout::parse(&text).unwrap();
        let ps = l.for_monitor("x", "x");
        assert_eq!(ps.len(), 1);
        assert_eq!((ps[0].0.clone(), ps[0].1.size.as_deref()), (ws.clone(), Some("large")));
        assert!(set_position(&mut doc, &ws, "nyx.clock/large", Anchor::Bottom, [0, 24]));
        assert!(move_scope(&mut doc, &ws, &Scope::All, "nyx.clock/large"));
        let l = Layout::parse(&doc.to_string()).unwrap();
        assert_eq!(l.for_monitor("x", "x")[0].1.anchor, Anchor::Bottom);
        let mon = Scope::MonitorWorkspace("DP-1".into(), "2".into());
        assert_eq!(mon.with_workspace(None), Scope::Monitor("DP-1".into()));
        assert!(move_scope(&mut doc, &Scope::All, &mon, "nyx.clock/large"));
        assert!(doc.to_string().contains("[[monitor.DP-1.workspace.2.widget]]"), "{doc}");
    }

    #[test]
    fn adds_and_removes() {
        let mut doc = DocumentMut::new();
        let mut p = Placement::new("nyx.stats/graphs");
        p.anchor = Anchor::BottomLeft;
        p.size = Some("large".into());
        let mon = Scope::Monitor("HEADLESS-3".into());
        add(&mut doc, &mon, &p);
        add(&mut doc, &Scope::All, &Placement::new("nyx.clock/large"));
        let text = doc.to_string();
        assert!(text.contains("[[monitor.HEADLESS-3.widget]]"), "{text}");
        let l = Layout::parse(&text).unwrap();
        assert_eq!(l.version, 1);
        assert_eq!(l.for_monitor("HEADLESS-3", "HEADLESS-3").len(), 2);
        // Adding again moves it.
        p.offset = [0, 0];
        add(&mut doc, &mon, &p);
        assert_eq!(doc.to_string().matches("nyx.stats/graphs").count(), 1);
        assert!(remove(&mut doc, &mon, "nyx.stats/graphs"));
        assert!(!remove(&mut doc, &mon, "nyx.stats/graphs"));
        assert!(set_free_size(&mut doc, &Scope::All, "nyx.clock/large", 400, 216));
        let free = &Layout::parse(&doc.to_string()).unwrap().for_monitor("x", "x")[0].1;
        assert_eq!((free.size.as_deref(), free.width, free.height), (Some("free"), Some(400), Some(216)));
        assert!(set_size(&mut doc, &Scope::All, "nyx.clock/large", "large"));
        assert!(!doc.to_string().contains("width"), "a named size drops the free one");
        assert_eq!(Layout::parse(&doc.to_string()).unwrap().for_monitor("HEADLESS-3", "HEADLESS-3").len(), 1);
    }
}
