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
    /// A size name; none: the part's default.
    #[serde(default)]
    pub size: Option<String>,
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
}

/// Which list a placement comes from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    All,
    /// A monitor's stable id or connector, as written in the file.
    Monitor(String),
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

    /// What shows on one monitor, each part once.
    pub fn for_monitor(&self, stable_id: &str, connector: &str) -> Vec<(Scope, Placement)> {
        let key = self.monitor_key(stable_id, connector);
        let mut seen = HashSet::new();
        let mut out = vec![];
        if let Some(g) = self.monitor.get(&key) {
            for p in &g.widget {
                if seen.insert(p.part.clone()) {
                    out.push((Scope::Monitor(key.clone()), p.clone()));
                }
            }
        }
        for p in &self.all.widget {
            if seen.insert(p.part.clone()) {
                out.push((Scope::All, p.clone()));
            }
        }
        out
    }

    /// Every plugin with a widget anywhere, monitors that aren't plugged in included.
    pub fn plugins_placed(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .all
            .widget
            .iter()
            .chain(self.monitor.values().flat_map(|g| g.widget.iter()))
            .map(|p| p.split().0.to_string())
            .collect();
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
    let parent: &mut Table = match scope {
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
        assert_eq!(Layout::parse(&doc.to_string()).unwrap().for_monitor("HEADLESS-3", "HEADLESS-3").len(), 1);
    }
}
