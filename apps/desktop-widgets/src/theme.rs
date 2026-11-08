//! The shared Nyx theme, with the widgets' own sheet. Widgets follow the theme every
//! Nyx app shares (`~/.config/nyx/theme.toml`), as the bar does.

use nyx_theme::{Style, ThemeMode, ThemePrefs};

pub fn install() {
    nyx_theme::use_design_2();
    nyx_theme::use_shell_scope();
    nyx_theme::install(Style {
        window_class: "nyx-desktop-widgets",
        base_skip: &[],
        css: include_str!("style.css"),
        no_glow_css: "",
        extra_colors: None,
        prefs: || {
            let suite = nyx_theme::read_suite();
            ThemePrefs {
                mode: suite.as_ref().map(|s| s.mode.clone()).unwrap_or(ThemeMode::Omarchy),
                theme: suite.map(|s| s.theme).unwrap_or_default(),
                glow: false,
                reduce_motion: false,
            }
        },
        set_theme: None,
    });
}
