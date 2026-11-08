# Theming

Study of how other UI frameworks handle theming and what dfui should take from
them. The proposed design is under "Proposed design"; nothing here is built yet.

## Summary

dfui should add a `Theme` of 12 named color roles, inherited through the
`provide` mechanism it already has. This is the Compose and Flutter model cut
down to what two real apps use.

- Widgets store a role and resolve it while painting, which sidesteps
  constructors having no access to the tree.
- `Theme::light()` stays the default, so existing apps and goldens don't change.
- A dark app becomes one line: `Theme { accent: GOLD, ..Theme::dark() }`.
- Skipped for now: typography, per-component themes, tonal palettes and animated
  switching.

Three decisions are still open, listed under "Open questions".

## The problem today

dfui's look is spread over 46 lines of hardcoded colors in 13 widget files, all
assuming a light app. Both real apps are dark, so they work around it at nearly
every call.

| Finding | Evidence (dfui at `3bd6a18`) |
| --- | --- |
| Accent has no single source | `0x3b6fe0` is written separately in `button`, `toggle`, `slider`, `radio`, `segmented`, `text_input` and `progress` |
| Ink has no single source | `INK = 0x1d2030` is declared four times: `segmented`, `table`, `tree`, `overlay` |
| Overlays are locked to white | `dialog`, `bottom_sheet` and `dropdown` paint white with dark text, inside dark apps |
| Some colors can't be overridden | `tabs` and `segmented` expose no color for label, track or divider |
| Only the window background is app-level | `App::background()` is the one theme-like hook |

The cost shows in the apps:

- **bbteam** declares its own palette (`BG`, `SURFACE`, `INK`, `MUTED`, `GOLD`,
  `DANGER`) and makes 130 `.color(..)` calls.
- **The music app** repeats the pattern with `BG`, `INK`, `MUTED`, `ACCENT` and
  27 `.color(..)` calls.
- **bbteam copies dfui's light values back** (`DARK = 0x1d2030`,
  `KEY = 0xe8e9ef`) so its content stays readable on dfui's white overlays.

One constraint shapes the design. Constructors such as `button()` and the
`stateful` closure run with no access to the tree, so they can't read an
inherited value. `provide` and `Scope` already deliver inherited values, but
only to `Widget::expand`, which is how `Label` reads a button's press state.

## How other frameworks do theming

The tree-based frameworks converge on one pattern: named color roles, inherited
down the tree, overridable for a subtree. Rust's immediate-mode and Elm-style
toolkits have no comparable tree scope, or pin the look at build time.

| Framework | What a theme holds | How widgets get it | Override for a subtree | Override one widget |
| --- | --- | --- | --- | --- |
| [Flutter](https://api.flutter.dev/flutter/material/Theme-class.html) | [`ThemeData`](https://api.flutter.dev/flutter/material/ThemeData-class.html): a 45-color `colorScheme`, text theme, one theme per component | `Theme.of(context)`, nearest ancestor | Nest a `Theme` | Widget parameters, or a per-component theme |
| [Compose Material 3](https://developer.android.com/develop/ui/compose/designsystems/material3) | Color scheme, typography, shapes | [`CompositionLocal`](https://developer.android.com/develop/ui/compose/compositionlocal), read as `MaterialTheme.colorScheme` | `CompositionLocalProvider`, or nest `MaterialTheme` | `ButtonDefaults.buttonColors(..)` |
| SwiftUI (approximate) | Separate environment values such as `tint` and `colorScheme` | `@Environment` | `.environment(..)` and `.tint(..)` | Modifiers on the view |
| [CSS custom properties](https://developer.mozilla.org/en-US/docs/Web/CSS/Guides/Cascading_variables/Using_custom_properties) | Any `--name` value | `var(--name, fallback)`, inherited | Redefine on an element | Redefine or set the property directly |
| [egui](https://docs.rs/egui/latest/egui/style/struct.Visuals.html) | `Visuals`: 36 fields, with [`Widgets`](https://docs.rs/egui/latest/egui/style/struct.Widgets.html) styled per interaction state | Held by the `Context` and `Ui` | `Ui::visuals_mut` | Same, in a narrow `Ui` |
| [iced](https://docs.rs/iced/latest/iced/theme/index.html) | `Theme` over a six-color [`Palette`](https://docs.rs/iced_core/latest/iced_core/theme/palette/struct.Palette.html): background, text, primary, success, warning, danger | Style functions and a per-widget `Catalog` trait | Not confirmed from the pages read | [Named styles](https://docs.rs/iced_widget/latest/iced_widget/button/index.html) such as `primary` and `danger`, or a `StyleFn` |
| [Slint](https://docs.slint.dev/latest/docs/slint/reference/std-widgets/style/) | Compiled style such as `fluent` or `material`, with `-light` and `-dark` variants | `Palette` and `StyleMetrics` globals | Not offered on the page read | Component properties |

Details that matter for dfui:

- **Paired colors.** Material 3 pairs each background with an "on" color.
  Compose's `LocalContentColor` makes text inside a `Surface` contrast
  automatically.
- **Good defaults.** Google's guidance is that a `CompositionLocal` should have
  a good default, so tests and previews work without setup. It should also be
  used only for tree-scoped concerns.
- **Rebuild cost.** Compose offers `staticCompositionLocalOf` for values that
  rarely change: a change recomposes the whole subtree, and reads aren't tracked.
- **App-defined tokens.** Flutter has `ThemeExtension`, keyed by type. dfui's
  `provide` is already keyed by type.
- **Semantic names.** MDN recommends names like `--main-text-color` over names
  that describe the value.

The SwiftUI row is from general knowledge, because Apple's reference page body
wasn't readable when this was written.

## What to take and what to skip

Take Compose's model at a tenth of its size: a small set of named roles,
inherited through the existing `provide`, with content colors paired to
surfaces.

| Idea | Seen in | Verdict for dfui | Why |
| --- | --- | --- | --- |
| Named color roles | Compose, Flutter, iced, CSS | Take | Fixes the seven copies of accent and four of ink |
| Inherited, nearest ancestor wins | Flutter, Compose, SwiftUI, CSS | Take | `provide` and `Scope` already work this way |
| Paired content color | Compose | Take | Removes most of the `.color(INK)` calls in both apps |
| Light default without setup | Compose guidance | Take | Existing apps and goldens keep working |
| Light and dark presets | All | Take | Both real apps are dark |
| App-defined tokens | Flutter `ThemeExtension` | Already covered | Apps call `provide` with their own type |
| Small palette | iced (6 colors) | Take the size | The audit needs about 12 roles; Flutter's 45 is far more than two apps use |
| Per-component themes | Flutter | Skip | `.color()` on the widget covers the need so far |
| Per-state style tables | egui | Skip | dfui derives hover and press from the base color |
| Tonal palettes, dynamic color | Material 3 | Skip | Needs a color algorithm and no app has asked |
| Animated theme change | Flutter `AnimatedTheme` | Skip for now | Nothing switches themes at runtime yet |
| Compile-time styles | Slint | Reject | dfui needs per-subtree overrides |

## Proposed design

A `Theme` struct of named colors, delivered by `provide`, with each widget
storing a role instead of a color and resolving it at paint time.

### Roles

| Role | Used for | Today (light) | bbteam equivalent |
| --- | --- | --- | --- |
| `background` | Window | `WHITE` | `BG` |
| `surface` | Dialog, sheet, menu | `WHITE` | `SURFACE` |
| `ink` | Text on background and surface | `0x1d2030` | `INK` |
| `muted` | Secondary text | `0x6b7080` | `MUTED` |
| `accent` | Buttons, toggles, focus | `0x3b6fe0` | `GOLD` |
| `on_accent` | Text on accent | Black, from `text` | `BG` |
| `selected` | Chosen menu row | `0xe8eefc` | None yet |
| `track` | Slider, progress, tab track | `0xd5d8e0`, `0xe8e9ef` | `CHIP` |
| `outline` | Input border, radio ring, toggle off | `0xc9ccd6`, `0x9a9fb0` | `EDGE` |
| `divider` | Tables, tabs | `0xdfe1e8` | None yet |
| `inverse` | Snackbar, tooltip | `0x323232`, `0x404040` | None yet |
| `danger` | Destructive actions | None | `DANGER` |

### API sketch

```rust
#[derive(Clone)]
pub struct Theme { pub background: Color, pub surface: Color, pub ink: Color, /* roles above */ }

impl Theme {
    pub fn light() -> Self;
    pub fn dark() -> Self;
}

pub enum Role { Background, Surface, Ink, Muted, Accent, /* roles above */ }

pub enum Ink { Fixed(Color), Role(Role), Content }
impl From<Color> for Ink { /* Fixed */ }
impl From<Role> for Ink { /* Role */ }

pub trait App {
    fn view(&self) -> Element;
    fn theme(&self) -> Theme { Theme::light() }
}

// One app, one line:
fn theme(&self) -> Theme { Theme { accent: GOLD, ..Theme::dark() } }

// A subtree, using what exists today:
provide(Theme::light(), receipt_view())

// A widget, unchanged:
button(text("Delete")).color(DANGER)
```

### How a color reaches the screen

1. `App::theme` seeds the root of the `provide` stack, so a `Theme` is always
   found.
2. The layout walk records the nearest `Theme` on each node, which is one `Rc`
   clone per node.
3. `PaintCx` exposes that theme, so widgets resolve their `Ink` while painting.
4. A theme change comes through the normal rebuild, and the existing scene diff
   repaints what changed.

Step 3 is what removes the constructor problem: `button()` stores a role and
never needs the tree. Composites work the same way, so `dropdown` writes
`.bg(Role::Surface)` inside its `stateful` closure.

### Content color

Buttons and surfaces provide the color their text should use, and a `Label` with
no explicit color reads it. `Button` already provides `Emphasis` to its labels,
so this extends that value. `Widget::provide` returns a single value per widget,
which is enough for now.

### Non-goals

Typography, spacing and corner radius stay out of the first version. Neither app
has asked to change them, and each can be added as a `Theme` field later.

## Rollout

The work splits into four steps, each landing on its own, and the first changes
no pixels.

1. **Plumbing, no visual change.** Add `Theme`, `Ink`, `App::theme` and
   `PaintCx` access, then move each widget's constants to roles. All 20 goldens
   must pass untouched.
2. **Dark overlays.** Fill in `Theme::dark()` and resolve `dialog`,
   `bottom_sheet`, `dropdown`, `tabs` and `segmented` through it. Add dark
   goldens for each.
3. **Content color.** Labels pick up the color of the surface or button around
   them.
4. **Migrate the apps.** Convert the music app, then bbteam, deleting their
   workaround constants. Put bbteam in git first.

Tests to add:

- A preset contrast check: every text role against its background reaches
  [4.5:1](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html), the
  WCAG AA minimum for normal text. Disabled controls are exempt.
- A nested `provide(Theme::light(), ..)` inside a dark app resolves to light,
  and only inside that subtree.

## Open questions

- [ ] Merge the two `track` values (`0xd5d8e0`, `0xe8e9ef`) and the two `outline`
      values into one each? Merging means re-blessing goldens once.
- [ ] Should `Theme::dark()` follow the system setting, or stay an explicit app
      choice? Neither app needs system following today.
- [ ] Should `on_accent` default to black to match today's goldens, or white for
      contrast on `0x3b6fe0`?
- [ ] Is colors-only right for the first version, or should corner radius and
      fonts be in `Theme` from the start?
