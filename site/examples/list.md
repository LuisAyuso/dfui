# List

`list(model, row)` builds only the rows near the viewport, so thousands of
items cost the same as a screenful. Rows are matched by key: the first
visible one stays in place when items change above it, and inserts, removals
and moves slide.

A `Vec` of `Keyed` items is a model, and so is a shared `Rc<RefCell<..>>`.
Like `scroll`, a list needs a bounded height: give every flex box between it
and the window `.fill()`.
