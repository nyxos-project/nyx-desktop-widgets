//! Up next: the coming events from Nyx Calendar's agenda (`agenda.sqlite`, which
//! `nyx-calendard` writes: a published read-only contract), as a desktop widget. Read
//! again when the file changes and as the minute turns, so "in 5 min" stays true, and
//! not at all while the widget is covered. Hidden until Calendar has run.

use chrono::{Datelike, Local, TimeZone};
use nyx_plugin_sdk::protocol::Widget;
use nyx_plugin_sdk::view::*;
use nyx_plugin_sdk::{Input, Plugin};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, PartialEq)]
struct Event {
    title: String,
    color: Option<String>,
    location: String,
    starts: i64,
    ends: i64,
    all_day: bool,
}

fn agenda_path() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"));
    data.join("nyx-calendar/agenda.sqlite")
}

/// The events not over by `now`, soonest first, each once; `None` without an agenda.
fn read(path: &std::path::Path, now: i64) -> Option<Vec<Event>> {
    if !path.exists() {
        return None;
    }
    let db = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let mut stmt = db
        .prepare(
            "SELECT title, color, location, starts, ends, all_day FROM events \
             WHERE ends > ?1 ORDER BY all_day DESC, starts, title LIMIT 40",
        )
        .ok()?;
    let rows = stmt
        .query_map([now], |r| {
            Ok(Event {
                title: r.get(0)?,
                color: r.get::<_, Option<String>>(1)?.filter(|c| c.len() == 7 && c.starts_with('#')),
                location: r.get(2)?,
                starts: r.get(3)?,
                ends: r.get(4)?,
                all_day: r.get::<_, i64>(5)? != 0,
            })
        })
        .ok()?;
    let mut events: Vec<Event> = Vec::new();
    for e in rows.flatten() {
        // Holidays come from several calendars: one of each.
        if let Some(prev) = events.iter_mut().find(|p| p.title == e.title && p.starts == e.starts) {
            if prev.color.is_none() {
                prev.color = e.color;
            }
            continue;
        }
        events.push(e);
    }
    events.sort_by(|a, b| a.starts.cmp(&b.starts).then(b.all_day.cmp(&a.all_day)));
    events.truncate(8);
    Some(events)
}

/// When an event is, said briefly: "in 12 min", "14:00", "Tomorrow 09:00", "Wed",
/// "All day" (as the Control Center's Up next says it).
fn when(e: &Event, now: i64) -> String {
    let (Some(start), Some(today)) = (Local.timestamp_opt(e.starts, 0).single(), Local.timestamp_opt(now, 0).single()) else {
        return String::new();
    };
    let days = (start.date_naive() - today.date_naive()).num_days();
    if e.all_day {
        return match days {
            ..=0 => "All day".into(),
            1 => "Tomorrow".into(),
            2..=6 => start.format("%A").to_string(),
            _ => start.format("%b %-d").to_string(),
        };
    }
    if e.starts <= now {
        return "Now".into();
    }
    let mins = (e.starts - now) / 60;
    if mins < 60 {
        return format!("in {} min", mins.max(1));
    }
    let time = start.format("%H:%M").to_string();
    match days {
        0 => time,
        1 => format!("Tomorrow {time}"),
        2..=6 => format!("{} {time}", start.format("%a")),
        _ => format!("{} {}", start.format("%b"), start.day()),
    }
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
    t.push('…');
    t
}

fn view(w: &Widget, events: &[Event], now: i64) -> Node {
    let large = w.size == "large";
    let head = hbox([icon("x-office-calendar-symbolic").color("danger"), label("Up next").class("title")]).spacing(8);
    let mut rows = vec![head];
    if events.is_empty() {
        rows.push(label("Nothing coming up").color("dim_text").margin(12, 0, 0, 0));
    }
    let width = ((w.width - 32 - 14) / 8).max(12) as usize;
    for e in events.iter().take(if large { 5 } else { 2 }) {
        let bar = vbox([]).size(3, 34).background(e.color.as_deref().unwrap_or("accent"));
        let sub = if e.location.is_empty() { when(e, now) } else { format!("{} · {}", when(e, now), e.location) };
        let text = vbox([
            markup(format!("<b>{}</b>", escape(&shorten(&e.title, width)))),
            label(shorten(&sub, width + 4)).color("dim_text").font_size(12.0),
        ])
        .spacing(1);
        rows.push(hbox([bar, text]).spacing(10));
    }
    vbox(rows).spacing(if large { 10 } else { 8 })
}

fn main() {
    let (plugin, inputs) = Plugin::<()>::start();
    let tick = plugin.data();
    // Look at the file every few seconds (cheap: its modification time), and draw again
    // as the minute turns.
    std::thread::spawn(move || {
        loop {
            tick(());
            std::thread::sleep(Duration::from_secs(5));
        }
    });
    let path = agenda_path();
    let mut widgets: Vec<Widget> = plugin.widgets().to_vec();
    let mut visible = true;
    let mut last: Option<(Option<SystemTime>, i64)> = None;
    let mut shown: Option<Option<Vec<Event>>> = None;
    let mut draw = |widgets: &[Widget], force: bool, last: &mut Option<(Option<SystemTime>, i64)>| {
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        let key = (std::fs::metadata(&path).and_then(|m| m.modified()).ok(), now / 60);
        if !force && last.as_ref() == Some(&key) {
            return;
        }
        *last = Some(key);
        let events = read(&path, now);
        if !force && shown.as_ref() == Some(&events) && now % 60 >= 5 {
            return;
        }
        for w in widgets.iter().filter(|w| w.item == "card") {
            let it = match &events {
                Some(ev) => item("card").monitor(&w.monitor).view(view(w, ev, now)).on_click(&["nyx-calendar"]),
                None => item("card").monitor(&w.monitor).hidden(true),
            };
            plugin.item(it);
        }
        shown = Some(events);
    };
    draw(&widgets, true, &mut last);
    for input in inputs {
        match input {
            Input::Data(()) if visible => draw(&widgets, false, &mut last),
            Input::Visible(v) => {
                if v && !visible {
                    draw(&widgets, false, &mut last);
                }
                visible = v;
            }
            Input::Widgets(w) => {
                widgets = w;
                draw(&widgets, true, &mut last);
            }
            Input::Shutdown => break,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        Local.with_ymd_and_hms(y, mo, d, h, mi, 0).single().unwrap().timestamp()
    }

    fn ev(starts: i64, all_day: bool) -> Event {
        Event { title: "x".into(), color: None, location: String::new(), starts, ends: starts + 3600, all_day }
    }

    #[test]
    fn says_when() {
        let now = at(2026, 11, 3, 9, 0);
        assert_eq!(when(&ev(at(2026, 11, 3, 9, 12), false), now), "in 12 min");
        assert_eq!(when(&ev(at(2026, 11, 3, 8, 30), false), now), "Now");
        assert_eq!(when(&ev(at(2026, 11, 3, 14, 0), false), now), "14:00");
        assert_eq!(when(&ev(at(2026, 11, 4, 9, 0), false), now), "Tomorrow 09:00");
        assert_eq!(when(&ev(at(2026, 11, 5, 9, 0), false), now), "Thu 09:00");
        assert_eq!(when(&ev(at(2026, 11, 3, 0, 0), true), now), "All day");
        assert_eq!(when(&ev(at(2026, 11, 20, 0, 0), true), now), "Nov 20");
    }

    #[test]
    fn reads_an_agenda() {
        let dir = std::env::temp_dir().join(format!("upnext-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agenda.sqlite");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE events (title TEXT, calendar_name TEXT, color TEXT, location TEXT, starts INTEGER, ends INTEGER, all_day INTEGER);
             INSERT INTO events VALUES ('Old', 'Work', '#ff0000', '', 0, 10, 0);
             INSERT INTO events VALUES ('Standup', 'Work', '#3366ff', 'Room 2', 200, 300, 0);
             INSERT INTO events VALUES ('Holiday', 'A', NULL, '', 100, 1000, 1);
             INSERT INTO events VALUES ('Holiday', 'B', '#00aa00', '', 100, 1000, 1);",
        )
        .unwrap();
        let events = read(&path, 50).unwrap();
        assert_eq!(events.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), vec!["Holiday", "Standup"]);
        assert_eq!(events[0].color.as_deref(), Some("#00aa00"), "a holiday once, with a colour");
        assert!(read(&dir.join("missing.sqlite"), 0).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
