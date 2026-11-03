# Data models

How lists, tables and trees get their data without the widgets knowing
where it lives, and how they stay cheap and animate when it changes. The
models are in `src/model.rs`; the widgets are `list`, `table` and `tree_view`
(`src/widgets/{list,table,tree}.rs`). The demo's Cards page shows all three.

## In other frameworks

| Framework | The data | Item → widget | How changes are reported |
|---|---|---|---|
| Qt model/view | `QAbstractItemModel`: `rowCount`, `data(index, role)`, parent indices for trees | delegate | `rowsInserted`, `dataChanged(range)` signals |
| GTK4 | `GListModel`: `n_items`, `get_item(i)`; trees via `TreeListModel` | `ListItemFactory` | `items-changed(pos, removed, added)` |
| Slint | `trait Model { row_count, row_data, … }`, `VecModel` | repeater | model tracker |
| UIKit / AppKit | data source; diffable data sources hold snapshots of ids | reused cell | the framework diffs the snapshots' ids |
| Android | `RecyclerView.Adapter` + `DiffUtil`; Compose `LazyColumn { items(list, key) }` | `ViewHolder` / composable | diff by key |
| Flutter | anything | `ListView.builder(itemCount, itemBuilder)` | rebuild |
| SwiftUI | `Identifiable` collections; `Table` with `TableColumn`s | row view | rebuild, diff by id |
| React | arrays + `key`; react-window `itemCount` + `render(i)`; TanStack Table (headless) | render function | rebuild, diff by key |

They all make the same choices: index-based access so a list only asks for
the rows it shows; stable keys so row state and animations follow items;
rendering in a builder outside the model; selection and expansion held by
key outside the model; sorting and filtering as a reordering of indices.
They differ in how changes are reported: fine-grained events (Qt, GTK,
Slint) or a rebuild diffed by key (React, SwiftUI, Compose).

## dfui's shape

Access is Slint/GTK-shaped, changes are SwiftUI-shaped:

```rust
pub trait ListModel {
    type Item;
    fn len(&self) -> usize;
    fn with<R>(&self, i: usize, f: impl FnOnce(&Self::Item) -> R) -> R;
    fn key(&self, i: usize) -> u64;
}

pub trait TreeModel {
    type Item;
    fn children(&self, parent: Option<u64>, f: &mut dyn FnMut(u64)); // None: roots
    fn with<R>(&self, key: u64, f: impl FnOnce(&Self::Item) -> R) -> R;
    fn has_children(&self, key: u64) -> bool;
}
```

- `Vec<T: Keyed>` is a `ListModel`; `Rc<M>` and `RefCell<M>` forward, so an
  app can share `Rc<RefCell<Vec<T>>>` with a background job. `Tree<T>` is an
  in-memory `TreeModel`.
- `with` takes a closure, so a model behind a `RefCell` borrows only for
  the call.
- There are no change events. The app changes its data and the tree is
  rebuilt (a `stateful` update, a `watch` on a `Notifier`); the list diffs
  the new keys against the rows it showed. dfui rebuilds and diffs anyway,
  so events would only add a contract for models to get wrong.
- Items must be `Clone`: each built row keeps a copy so it can still be
  drawn after the item is gone (see "Removals"). Only rows near the viewport
  are copied; make large items cheap to clone (`Rc`).

```rust
list(items, |it: &Song| song_row(it)).fill()

table(items, vec![
    Column::new("Title", |s: &Song| text(&s.title)).sort_by(|a, b| a.title.cmp(&b.title)),
    Column::new("Size", |s: &Song| text(s.size())).width(96.0),
]).fill()

tree_view(library, |node: &Node, row: &Row| node_label(node)).fill()
```

## One virtualized widget

`List` is the only widget that scrolls rows. `table` and `tree_view` are
adapters that produce rows for it (`Rows`: count, key, builder per index):

- **table:** rows in sort order (an index permutation computed per build,
  the model untouched), each a row of cells sized like its header. The sort
  column and direction are the table's `stateful` state.
- **tree_view:** `flatten(model, open)` walks the open nodes depth first into
  `Row { key, depth, expanded, has_children }`. Expanded keys are the tree
  view's state, not the model's, as in GTK and Qt. A tree is a list of rows
  with an indent and an arrow.

Keys must be unique within a list, and for a tree across the whole tree.

## Virtualization

`List` builds its rows in `expand` from its own state:

- **Heights** are measured, not declared: each row reports its height into
  a cache keyed by key when it is placed. Rows not yet measured count as the
  average measured height. Row content can be any height and can change.
- **Built range:** from the offset and viewport height (from the last
  placement), the rows within one viewport above and two below, using the
  cached heights. Spacers above and below stand in for the rest, so the
  scroll extent covers the whole list.
- **Scrolling** is placement-only while the visible rows stay inside the
  built range. Once fewer than half a viewport of built rows is left on
  either side, the list asks for a rebuild while painting
  (`PaintCx::request_rebuild`), so a fling rebuilds once a screen or so,
  not every frame.
- **Anchoring:** the scroll position is stored as the first visible row's
  key and how far it sits above the top, not as pixels. Each build puts
  that row back where it was, so inserts and removals above the viewport,
  and reloads, leave the view still. If the anchor row is gone, its old
  index stands in.
- **Estimate errors:** when rows built from estimates measure differently,
  the next paint moves the offset by the difference so the anchor stays put.
  It is corrected a frame late, so that frame shows the error once.

The cost of a rebuild is a key and height lookup per item (to place the
anchor and compute the range) plus building the rows near the viewport;
the rest of the list costs nothing.

## Animations

On each build the list compares the rows it painted last (key and screen
position) with the new ones:

- **Moves** (sorts, rows above inserted or removed while the row stays
  visible): FLIP. The new layout is applied at once, and each moved row is
  drawn offset by its old minus its new position, easing to zero. Rows
  start from wherever a running animation had them.
- **Inserts:** a new key on screen slides in from the right.
- **Removals:** a removed row on screen becomes a ghost. Its saved builder
  draws it again, positioned absolutely where it was, sliding out to the
  right behind the rows that take its place. Ghosts are dropped after the
  animation (a rebuild requested from paint), or when their key comes back.
- **Reloads:** if fewer than half of the visible rows survive a change, the
  list cuts to the new rows without animating.

Rows outside the viewport never animate. Offsets are applied through
`Widget::scroll` on a per-row slot that doesn't clip (`Widget::clips`), so
the animations need no relayout per frame. Only rows on screen are diffed.
`list(..).animate(false)` turns animations off.

Ghosts are not cut off from input: a tap on one while it slides out
reaches its buttons, which act on an item that is already gone.

## Not done

- Per-item change notification (Qt's `dataChanged`): a change rebuilds the
  whole view, and the list rebuilds the rows near the viewport.
- Selection models; filtering adapters; horizontal virtualization for wide
  tables; lazy tree children with a loading row; paged remote data
  (`on_range`).
- After a sort, the anchor keeps the row that was first visible at the top,
  rather than scrolling to the top.
- Huge, fully expanded trees: flattening walks every open node on every
  build; per-node visible counts (GTK's `TreeListModel`) would make it
  O(log n) per lookup.
