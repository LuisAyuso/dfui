# Tree view

`tree_view(model, row)` shows a `TreeModel` as an indented, virtualized
list. The arrow before a node with children expands or collapses it, and
`row` draws the rest of each line. `Tree` is an in-memory model of `Keyed`
items. What is expanded is view state, kept apart from the model.
