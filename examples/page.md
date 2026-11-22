# Page

`page(title)` lays out a screen: a header row, then the body padded and
filling the window. `.back(f)` puts an arrow before the title and runs `f`
on Escape and the Android back button too; `.menu(f)` puts a menu button
there instead, e.g. to show a `drawer`. The title is any widget: `heading`
for bold text, or `tabs`.

The arrow, menu and close shapes are in `dfui::icons`, for `icon` and
`icon_button` elsewhere.
