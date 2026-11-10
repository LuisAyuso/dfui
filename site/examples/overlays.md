# Overlays

Overlays float above the rest of the tree without taking part in its
layout, aren't clipped by scroll views, and get input first.

`overlays(|ov| content)` hosts the ones opened from handlers:
`ov.show(cx, |me| dialog(..))` slides a `dialog`, `drawer`,
`bottom_sheet` or `snackbar` in, and `me.close(cx)` (or a tap outside a
modal one) slides it out and removes it once the animation ends. A
snackbar closes itself after four seconds.

`ov.confirm(cx, title, body, ok, on_ok)` asks before a destructive action.

A `dropdown` and a `tooltip` own their popup, so they need no host.
`overlay(child)` (covers the window) and `popup(child)` (below its parent)
are the building blocks underneath.
