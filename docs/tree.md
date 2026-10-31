# Flattened tree

How the runtime turns the `Element` tree into the flat node list that layout,
paint and dispatch work on, and how that list is cached between rebuilds.
For the overall frame loop, see `architecture.md`.

## Flattening

Layout, paint and dispatch all walk the same flat, pre-order `Vec<Node>`
(`layout.rs`). Indices line up with `Runtime::measured` and `Runtime::placed`,
so node `i`'s box is `placed[i]`.

```
  Element tree                              Vec<Node>  (pre-order)
                                            i  id     parent  end  w
  column ─┬─ scroll ── column ─┬─ card 1    0  a3f…   -       N    &column
          │                    ├─ card 2    1  91c…   0       N    &scroll
          │                    └─ …         2  0d7…   1       N    &column
                                            3  e40…   2       9    &card 1
                                            4  …      3       5    …
```

`end` is one past the node's last descendant, so a subtree is `i..end` and
the next sibling starts at `end`.

`flatten` builds this from scratch. For every node it:

1. hashes (parent id, key or child slot) into the node's `id`,
2. looks up or creates the node's `State` slot (a second hash),
3. calls `Widget::expand`, which builds a `stateful` subtree from its state,
4. pushes a `Node` that borrows the widget from the tree.

The hashing is the expensive part. At 1000 cards (about 11k nodes) one
`flatten` takes about 450 µs on desktop.

## The id cache

The tree only changes when `App::view` runs. Hover, press, scroll and caret
frames reuse the same tree, yet each one used to flatten it up to three times:

```
  wheel tick ──► dispatch ──► flatten          (find targets)
                    └─► reposition ──► flatten (place)
  redraw ──────────────────► scene ──► flatten (paint)
```

Now the runtime flattens once per tree and keeps only the ids
(`Runtime::ids`). Every other pass calls `walk`, which follows `children()`
in the same pre-order and pairs each widget with `ids[i]`. It rebuilds
`parent` and `end` along the way, so they don't need caching:

```
  App::view ─► set_root ─► flatten once ─► ids: Vec<u64>
                                             │
               dispatch ──── walk(root, ids) ┤
               relayout ──── walk(root, ids) ┤
               reposition ── walk(root, ids) ┤
               scene ─────── walk(root, ids) ┘
```

The widget references themselves can't be cached: `Node` borrows from
`root`, and `Runtime` owns `root`, so storing nodes would make the struct
borrow from itself. Re-walking costs a pointer chase per node, about 15% of
a `flatten`.

## Invalidation

There's no per-entry eviction. `ids` is one value: the ids of the current
`root`. `Runtime::set_root` is the only place `root` is replaced, and it
re-flattens straight away, so the two can't drift apart. The old list is
dropped whole.

This holds because the tree's shape can only change through `set_root`:

- `stateful` children are built in `expand` behind a `OnceCell`. That runs
  during the `flatten` in `set_root`, and the cell is never reset for that
  element.
- State updates and task results always end in `App::view`, which creates
  new elements with empty cells and goes through `set_root`.
- Hover, press, scroll and caret change state and positions, never the
  shape.

`walk` checks the node count against `ids` in debug builds, which catches a
tree changed without re-flattening.

`states` is separate and does need pruning: after a rebuild, slots whose id
isn't in the new `ids` are dropped.

## Measured

`cargo bench --bench cpu`, desktop, before → after:

| Bench | 10 cards | 100 cards | 1000 cards |
|-------|----------|-----------|------------|
| `frame/pointer_move` (dispatch only) | 6.6 → 2.5 µs | 61 → 24 µs | 653 → 282 µs |
| `frame/scroll` (dispatch + place + paint) | 47 → 36 µs | 453 → 343 µs | 5.49 → 4.35 ms |
| `frame/edit` (rebuild) | 95 → 94 µs | 715 → 683 µs | 11.2 → 11.1 ms |

Rebuild frames gain little: they still flatten the new tree once, and their
cost is dominated by `view`, layout and paint.

On the Pixel 10a (`scripts/android-bench.sh cpu`, A/B against the previous
commit): `frame/scroll` 765 → 631 µs at 100 cards and 10.2 → 8.4 ms at
1000; `frame/edit` unchanged.

## Not done

- Dispatch still offers each pointer event to every node (about 215 µs of
  the 282 µs at 1000 cards). A spatial index would need to keep the
  current behavior where widgets also see moves that miss them (hover off,
  drags that leave a scroll view), for example by tracking every widget hit
  since pointer down.
- A rebuild re-flattens the whole tree even when `view` changed one label;
  that belongs with partial rebuild in `TODO.md`.
