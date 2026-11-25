# Scroll

`scroll` clips its child to its own box and scrolls it vertically: wheel,
or drag with a fling that slows down after release. Give it a bounded
height (`grow` inside a column, or `height`) so there is something to
scroll.

`.horizontal()` scrolls sideways instead, like the shelf of cards at the
top: drag, a sideways touchpad swipe, or shift+wheel. A plain wheel over it
still scrolls the page. Its items need `.shrink(0.0)`, or the row squeezes
them into the box. `.reveal(span)` glides to keep a span in view when it
changes, e.g. the item with keyboard focus.
