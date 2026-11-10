use crate::render::Color;

/// The colors widgets draw with. Return one from `App::theme`, or `provide`
/// one to restyle a subtree; the default is a light look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    /// The window.
    pub background: Color,
    /// Dialogs, sheets, menus, input fields and thumbs.
    pub surface: Color,
    /// Text on `background` and `surface`.
    pub ink: Color,
    /// Secondary text.
    pub muted: Color,
    /// Buttons, active controls and focus.
    pub accent: Color,
    /// Text on `accent`.
    pub on_accent: Color,
    /// The chosen row of a menu.
    pub selected: Color,
    /// Behind sliders, progress bars and segments.
    pub track: Color,
    /// Input borders, unchecked toggles and radios.
    pub outline: Color,
    /// The line under table headers and tab bars.
    pub divider: Color,
    /// Snackbars and tooltips, which contrast with the window.
    pub inverse: Color,
    /// Text on `inverse`.
    pub on_inverse: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            background: Color::WHITE,
            surface: Color::WHITE,
            ink: Color::hex(0x1d2030),
            muted: Color::hex(0x6b7080),
            accent: Color::hex(0x3b6fe0),
            on_accent: Color::WHITE,
            selected: Color::hex(0xe8eefc),
            track: Color::hex(0xd5d8e0),
            outline: Color::hex(0x9a9fb0),
            divider: Color::hex(0xdfe1e8),
            inverse: Color::hex(0x323232),
            on_inverse: Color::WHITE,
        }
    }
}

/// A `Theme` color by name, resolved where the widget is painted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Role {
    /// `Theme::background`.
    Background,
    /// `Theme::surface`.
    Surface,
    /// `Theme::ink`.
    Ink,
    /// `Theme::muted`.
    Muted,
    /// `Theme::accent`.
    Accent,
    /// `Theme::on_accent`.
    OnAccent,
    /// `Theme::selected`.
    Selected,
    /// `Theme::track`.
    Track,
    /// `Theme::outline`.
    Outline,
    /// `Theme::divider`.
    Divider,
    /// `Theme::inverse`.
    Inverse,
    /// `Theme::on_inverse`.
    OnInverse,
}

impl Theme {
    /// The color this theme gives `role`.
    pub fn color(&self, role: Role) -> Color {
        match role {
            Role::Background => self.background,
            Role::Surface => self.surface,
            Role::Ink => self.ink,
            Role::Muted => self.muted,
            Role::Accent => self.accent,
            Role::OnAccent => self.on_accent,
            Role::Selected => self.selected,
            Role::Track => self.track,
            Role::Outline => self.outline,
            Role::Divider => self.divider,
            Role::Inverse => self.inverse,
            Role::OnInverse => self.on_inverse,
        }
    }
}

/// What a widget paints with: a fixed color, a theme role, or whatever
/// contrasts with the button or surface around it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Ink {
    /// This color, whatever the theme.
    Fixed(Color),
    /// The theme's color for this role.
    Role(Role),
    #[default]
    /// The content color of the nearest `button` or `container` with `content`;
    /// `Role::Ink` outside them.
    Content,
}

impl From<Color> for Ink {
    fn from(c: Color) -> Self {
        Ink::Fixed(c)
    }
}

impl From<Role> for Ink {
    fn from(r: Role) -> Self {
        Ink::Role(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contrast(a: Color, b: Color) -> f32 {
        let lin = |v: f32| match v <= 0.04045 {
            true => v / 12.92,
            false => ((v + 0.055) / 1.055).powf(2.4),
        };
        let l = |c: Color| 0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b);
        let (a, b) = (l(a), l(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn default_text_roles_reach_wcag_aa() {
        let t = Theme::default();
        for (name, fg, bg) in [
            ("ink/background", t.ink, t.background),
            ("ink/surface", t.ink, t.surface),
            ("muted/surface", t.muted, t.surface),
            ("on_accent/accent", t.on_accent, t.accent),
            ("on_inverse/inverse", t.on_inverse, t.inverse),
        ] {
            assert!(contrast(fg, bg) >= 4.5, "{name}: {}", contrast(fg, bg));
        }
    }
}
