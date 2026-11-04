//! Where a widget goes: an anchor (one of nine points on the screen) and an offset from
//! it, so widgets stay put when the resolution or scale changes.

use serde::{Deserialize, Serialize};

/// Offsets snap to this many logical pixels.
pub const GRID: i32 = 8;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    #[default]
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

/// Where along one axis: the start edge, the middle, or the end edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Start,
    Middle,
    End,
}

impl Anchor {
    pub const ALL: [Anchor; 9] = [
        Anchor::TopLeft,
        Anchor::Top,
        Anchor::TopRight,
        Anchor::Left,
        Anchor::Center,
        Anchor::Right,
        Anchor::BottomLeft,
        Anchor::Bottom,
        Anchor::BottomRight,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Anchor::TopLeft => "top-left",
            Anchor::Top => "top",
            Anchor::TopRight => "top-right",
            Anchor::Left => "left",
            Anchor::Center => "center",
            Anchor::Right => "right",
            Anchor::BottomLeft => "bottom-left",
            Anchor::Bottom => "bottom",
            Anchor::BottomRight => "bottom-right",
        }
    }

    pub fn parse(s: &str) -> Option<Anchor> {
        Anchor::ALL.into_iter().find(|a| a.name() == s)
    }

    pub fn sides(self) -> (Side, Side) {
        use Side::*;
        match self {
            Anchor::TopLeft => (Start, Start),
            Anchor::Top => (Middle, Start),
            Anchor::TopRight => (End, Start),
            Anchor::Left => (Start, Middle),
            Anchor::Center => (Middle, Middle),
            Anchor::Right => (End, Middle),
            Anchor::BottomLeft => (Start, End),
            Anchor::Bottom => (Middle, End),
            Anchor::BottomRight => (End, End),
        }
    }

    pub fn from_sides(x: Side, y: Side) -> Anchor {
        *Anchor::ALL.iter().find(|a| a.sides() == (x, y)).expect("nine anchors")
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect { x, y, width, height }
    }

    pub fn right(&self) -> i32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    pub fn intersect(&self, o: &Rect) -> Option<Rect> {
        let (x, y) = (self.x.max(o.x), self.y.max(o.y));
        let r = Rect::new(x, y, self.right().min(o.right()) - x, self.bottom().min(o.bottom()) - y);
        (!r.is_empty()).then_some(r)
    }

    pub fn translate(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.width, self.height)
    }
}

fn along(side: Side, area: i32, size: i32, offset: i32) -> i32 {
    match side {
        Side::Start => offset,
        Side::Middle => (area - size) / 2 + offset,
        Side::End => area - size - offset,
    }
}

/// The widget's top-left corner in an area, kept inside it where it fits.
pub fn place(area: (i32, i32), size: (i32, i32), anchor: Anchor, offset: (i32, i32)) -> (i32, i32) {
    let (sx, sy) = anchor.sides();
    let x = along(sx, area.0, size.0, offset.0);
    let y = along(sy, area.1, size.1, offset.1);
    (x.min(area.0 - size.0).max(0), y.min(area.1 - size.1).max(0))
}

pub fn snap(v: i32) -> i32 {
    (v as f64 / GRID as f64).round() as i32 * GRID
}

fn side_for(pos: i32, size: i32, area: i32) -> Side {
    let center = pos + size / 2;
    if center * 3 < area {
        Side::Start
    } else if center * 3 > area * 2 {
        Side::End
    } else {
        Side::Middle
    }
}

fn offset_for(side: Side, pos: i32, size: i32, area: i32) -> i32 {
    snap(match side {
        Side::Start => pos,
        Side::Middle => pos - (area - size) / 2,
        Side::End => area - size - pos,
    })
}

/// The anchor and offset for a widget dropped at `pos`: the anchor follows the third of
/// the screen its centre is in, so a widget dragged to the right edge stays right.
pub fn locate(area: (i32, i32), size: (i32, i32), pos: (i32, i32)) -> (Anchor, (i32, i32)) {
    let pos = (pos.0.min(area.0 - size.0).max(0), pos.1.min(area.1 - size.1).max(0));
    let (sx, sy) = (side_for(pos.0, size.0, area.0), side_for(pos.1, size.1, area.1));
    (Anchor::from_sides(sx, sy), (offset_for(sx, pos.0, size.0, area.0), offset_for(sy, pos.1, size.1, area.1)))
}

/// Whether `rects` together cover all of `target`.
pub fn covered(target: &Rect, rects: &[Rect]) -> bool {
    if target.is_empty() {
        return true;
    }
    let parts: Vec<Rect> = rects.iter().filter_map(|r| r.intersect(target)).collect();
    if parts.is_empty() {
        return false;
    }
    let mut xs: Vec<i32> = parts.iter().flat_map(|r| [r.x, r.right()]).chain([target.x, target.right()]).collect();
    let mut ys: Vec<i32> = parts.iter().flat_map(|r| [r.y, r.bottom()]).chain([target.y, target.bottom()]).collect();
    xs.sort_unstable();
    xs.dedup();
    ys.sort_unstable();
    ys.dedup();
    for wx in xs.windows(2) {
        for wy in ys.windows(2) {
            let (cx, cy) = (wx[0], wy[0]);
            if !parts.iter().any(|r| r.x <= cx && cx < r.right() && r.y <= cy && cy < r.bottom()) {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_round_trip() {
        for a in Anchor::ALL {
            assert_eq!(Anchor::parse(a.name()), Some(a));
            let (x, y) = a.sides();
            assert_eq!(Anchor::from_sides(x, y), a);
        }
        assert_eq!(Anchor::parse("middle"), None);
    }

    #[test]
    fn places_from_each_anchor() {
        let area = (1920, 1080);
        let size = (336, 160);
        assert_eq!(place(area, size, Anchor::TopLeft, (48, 48)), (48, 48));
        assert_eq!(place(area, size, Anchor::TopRight, (48, 48)), (1920 - 336 - 48, 48));
        assert_eq!(place(area, size, Anchor::BottomRight, (0, 0)), (1920 - 336, 1080 - 160));
        assert_eq!(place(area, size, Anchor::Center, (0, 0)), ((1920 - 336) / 2, (1080 - 160) / 2));
        assert_eq!(place(area, size, Anchor::Bottom, (-100, 24)), ((1920 - 336) / 2 - 100, 1080 - 160 - 24));
    }

    #[test]
    fn stays_put_across_resolutions() {
        let size = (160, 160);
        // Right-anchored widgets keep their distance from the right edge.
        for area in [(1280, 800), (1920, 1080), (2560, 1440)] {
            let (x, _) = place(area, size, Anchor::TopRight, (48, 48));
            assert_eq!(area.0 - (x + size.0), 48);
        }
    }

    #[test]
    fn keeps_widgets_on_screen() {
        assert_eq!(place((800, 600), (336, 336), Anchor::TopLeft, (700, 500)), (800 - 336, 600 - 336));
        assert_eq!(place((800, 600), (336, 336), Anchor::TopLeft, (-50, -50)), (0, 0));
        // Too big to fit: pinned to the top left.
        assert_eq!(place((200, 200), (336, 336), Anchor::Center, (0, 0)), (0, 0));
    }

    #[test]
    fn locates_by_thirds_and_snaps() {
        let area = (1920, 1080);
        let size = (336, 160);
        assert_eq!(locate(area, size, (51, 45)), (Anchor::TopLeft, (48, 48)));
        let (a, off) = locate(area, size, (1920 - 336 - 50, 47));
        assert_eq!((a, off), (Anchor::TopRight, (48, 48)));
        let (a, off) = locate(area, size, ((1920 - 336) / 2 + 3, (1080 - 160) / 2));
        assert_eq!((a, off), (Anchor::Center, (0, 0)));
        let (a, _) = locate(area, size, (10, 1080 - 170));
        assert_eq!(a, Anchor::BottomLeft);
        // Locating then placing lands on the grid near where it was dropped.
        for pos in [(100, 100), (1500, 900), (800, 450), (5, 1000)] {
            let (a, off) = locate(area, size, pos);
            let back = place(area, size, a, off);
            let pos = (pos.0.min(area.0 - size.0), pos.1.min(area.1 - size.1));
            assert!((back.0 - pos.0).abs() <= GRID && (back.1 - pos.1).abs() <= GRID, "{pos:?} -> {back:?}");
        }
    }

    #[test]
    fn coverage() {
        let w = Rect::new(100, 100, 200, 100);
        assert!(!covered(&w, &[]));
        assert!(covered(&w, &[Rect::new(0, 0, 1000, 1000)]));
        assert!(!covered(&w, &[Rect::new(0, 0, 250, 1000)]));
        // Two windows side by side cover it together.
        assert!(covered(&w, &[Rect::new(0, 0, 200, 1000), Rect::new(200, 0, 800, 1000)]));
        // A gap between them doesn't.
        assert!(!covered(&w, &[Rect::new(0, 0, 199, 1000), Rect::new(201, 0, 800, 1000)]));
        assert!(covered(&w, &[Rect::new(90, 90, 120, 120), Rect::new(200, 90, 120, 60), Rect::new(200, 150, 120, 60)]));
    }
}
