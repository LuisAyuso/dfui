# Stateful

`stateful` keeps local state `S` (any `Default` type) across rebuilds,
matched by tree position or `Element::key`. Its build closure gets the
state and a `Copy` handle; handlers call `cx.update(handle, |s| ..)` and the
subtree is rebuilt from the new state. `stateful_with(init, build)` starts
from a value instead, and its `S` needn't be `Default`.

Each card below owns its own `open` flag.
