# Table

`table(model, columns)` shows a model's items as rows, on top of a
virtualized `list`. `Column::new(title, cell)` describes one column;
`width` fixes it, otherwise columns share the space left. With `sort_by`,
tapping the header sorts by it, then reversed, then back to the model's order.
