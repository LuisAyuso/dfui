# Theming

How dfui colors its widgets, and how other UI frameworks approach the same
problem.

## What dfui does

A `Theme` is a plain struct of 12 named colors. Widgets refer to these colors
by role and resolve them when they paint, so one value restyles everything
under it. dfui ships a single light default; an application defines its own look
by building a `Theme`.

```rust
impl App for MyApp {
    fn view(&self) -> Element { /* ... */ }
    fn theme(&self) -> Theme {
        Theme {
            background: Color::hex(0x14130f),
            surface: Color::hex(0x24211b),
            ink: Color::hex(0xf4efe2),
            accent: Color::hex(0xe0b44c),
            on_accent: Color::hex(0x14130f),
            ..Theme::default()
        }
    }
}
```

### Roles

| Role | Used for | Default |
| --- | --- | --- |
| `background` | The window | `0xffffff` |
| `surface` | Dialogs, sheets, menus, input fields, thumbs | `0xffffff` |
| `ink` | Text on `background` and `surface` | `0x1d2030` |
| `muted` | Secondary text | `0x6b7080` |
| `accent` | Buttons, active controls, focus | `0x3b6fe0` |
| `on_accent` | Text on `accent` | `0xffffff` |
| `selected` | The chosen row of a menu | `0xe8eefc` |
| `track` | Behind sliders, progress bars, segments | `0xd5d8e0` |
| `outline` | Input borders, unchecked toggles and radios | `0x9a9fb0` |
| `divider` | Lines in tables and tabs | `0xdfe1e8` |
| `inverse` | Snackbars and tooltips | `0x323232` |
| `on_inverse` | Text on `inverse` | `0xffffff` |

The default theme's text roles reach the
[WCAG AA contrast of 4.5:1](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html)
against their backgrounds, and a test holds them to it.

### Setting colors

Color setters such as `.color()`, `.bg()`, `.border()` and `.accent()` take
anything that converts into an `Ink`:

```rust
pub enum Ink { Fixed(Color), Role(Role), Content }

button(text("Delete")).color(Color::hex(0xc0392b)); // one fixed color
container(body).bg(Role::Surface);                  // follows the theme
```

`Ink::Content` is the default for `text` and `icon`. It uses the color meant to
contrast with what surrounds the widget: `on_accent` inside a button, or
whatever a container declares with `.content(..)`. With no such ancestor it is
the theme's `ink`.

### Scope

The theme comes from `App::theme`. `provide` replaces it for a subtree, using
the same mechanism as any other provided value:

```rust
provide(Theme { accent: red, ..Theme::default() }, panel)
```

`App::theme` is read every frame, like `App::background`, whose default is the
theme's `background`. Application-specific colors need no theme support: an app
can `provide` a struct of its own and read it through `Scope`.

### How a color reaches the screen

Constructors such as `button()` and the closure of `stateful` run outside the
tree, so they can't look up an inherited value. Widgets therefore store an `Ink`
and resolve it at paint time:

1. Before painting, the runtime walks the nodes in pre-order. Each node takes
   its parent's theme, or the `Theme` it provides.
2. `PaintCx::theme` gives each widget its resolved theme, and `PaintCx::color`
   turns an `Ink` into a `Color`.
3. Hover, press and disabled looks are derived from the resolved color.

### What is left out

| Omitted | Reason |
| --- | --- |
| Built-in dark or other presets | Applications supply their own theme |
| Following the system dark setting | A theme is an explicit choice of the application |
| Fonts, spacing, corner radius | Colors only so far; each can become a field |
| Per-component themes | `.color()` on the widget covers individual overrides |
| Generated palettes | A theme is exactly the colors it lists |
| Animated theme changes | A new theme applies on the next frame |

## Other frameworks

The tree-based frameworks converge on one pattern: named color roles, inherited
down the tree, overridable for a subtree. Immediate-mode and Elm-style toolkits
tend to keep the style in a context or in style functions, and some fix it at
build time.

| Framework | What a theme holds | How widgets get it | Override for a subtree | Override one widget |
| --- | --- | --- | --- | --- |
| [Flutter](https://api.flutter.dev/flutter/material/Theme-class.html) | [`ThemeData`](https://api.flutter.dev/flutter/material/ThemeData-class.html): a 45-color `colorScheme`, text theme, one theme per component | `Theme.of(context)`, nearest ancestor | Nest a `Theme` | Widget parameters, or a per-component theme |
| [Compose Material 3](https://developer.android.com/develop/ui/compose/designsystems/material3) | Color scheme, typography, shapes | [`CompositionLocal`](https://developer.android.com/develop/ui/compose/compositionlocal), read as `MaterialTheme.colorScheme` | `CompositionLocalProvider`, or nest `MaterialTheme` | `ButtonDefaults.buttonColors(..)` |
| SwiftUI (approximate) | Separate environment values such as `tint` and `colorScheme` | `@Environment` | `.environment(..)` and `.tint(..)` | Modifiers on the view |
| [CSS custom properties](https://developer.mozilla.org/en-US/docs/Web/CSS/Guides/Cascading_variables/Using_custom_properties) | Any `--name` value | `var(--name, fallback)`, inherited | Redefine on an element | Redefine or set the property directly |
| [egui](https://docs.rs/egui/latest/egui/style/struct.Visuals.html) | `Visuals`: 36 fields, with [`Widgets`](https://docs.rs/egui/latest/egui/style/struct.Widgets.html) styled per interaction state | Held by the `Context` and `Ui` | `Ui::visuals_mut` | Same, in a narrow `Ui` |
| [iced](https://docs.rs/iced/latest/iced/theme/index.html) | `Theme` over a six-color [`Palette`](https://docs.rs/iced_core/latest/iced_core/theme/palette/struct.Palette.html): background, text, primary, success, warning, danger | Style functions and a per-widget `Catalog` trait | Not confirmed from the pages read | [Named styles](https://docs.rs/iced_widget/latest/iced_widget/button/index.html) such as `primary` and `danger`, or a `StyleFn` |
| [Slint](https://docs.slint.dev/latest/docs/slint/reference/std-widgets/style/) | Compiled style such as `fluent` or `material`, with `-light` and `-dark` variants | `Palette` and `StyleMetrics` globals | Not offered on the page read | Component properties |

The SwiftUI row is from general knowledge, because Apple's reference page body
wasn't readable when this was written.

Ideas from these that shaped dfui:

- **Paired colors.** Material 3 pairs each background with an "on" color, and
  Compose's `LocalContentColor` makes text inside a `Surface` contrast
  automatically. dfui's `on_accent` and `Ink::Content` follow this.
- **A good default.** Google's guidance is that a `CompositionLocal` should have
  a good default, so tests and previews work without setup, and should be used
  only for tree-scoped concerns.
- **Application tokens.** Flutter has `ThemeExtension`, keyed by type. dfui's
  `provide` is already keyed by type, so no extension point was needed.
- **Semantic names.** MDN recommends names like `--main-text-color` over names
  that describe the value.
- **A small palette.** iced manages with six base colors, against 45 in
  Flutter's `colorScheme`.

Pages opened on 2026-11-08 are linked above.
