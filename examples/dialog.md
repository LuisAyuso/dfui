# Dialog

`dialog(child)` is a modal card centered over the dimmed window. Show it
through an `overlays` host with `ov.show(cx, |me| ..)`; `me.close(cx)`, a
tap outside or Back closes it. `ov.confirm` is the common yes-or-no case.
