//! Nyx Desktop Widgets without GTK.
//!
//! - [`layout`]: `~/.config/nyx/desktop-widgets.toml`, read and edited.
//! - [`geometry`]: anchors and offsets, snapping, and whether windows cover a widget.

pub mod geometry;
pub mod layout;

pub use geometry::{Anchor, Rect};
pub use layout::{Background, Layout, Placement, Scope};
