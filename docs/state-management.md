# State management

Study of state architectures used by other UI frameworks and what each would
mean for dfui. Treats the current Elm model as one option, not a requirement.
The chosen design is under "Decision". Builds on `async-and-state.md` (runtime and
web constraints) and the "Global state" and "Partial rebuild" studies in
`TODO.md`.

## Goals

1. **Async.** State can change from futures on any runtime, and on web.
2. **Intuitive data flow.** It is clear who owns a piece of state, who reads
   it and who may change it. No all-to-all graph where any part can write any
   other part.
3. **Performance.** An update costs in proportion to what changed, not to the
   size of the UI.

## Where dfui is today

- `App` owns domain state and changes it through `Msg`; `stateful` widgets own
  ephemeral UI state, changed by their handlers via `Handle`.
- Every message reruns all of `App::view`, `flatten` and paint. Layout is
  incremental (taffy tree kept, only changed nodes dirtied) and the GPU redraws
  only damaged areas.
- Measured: 1000-card edit frame 11 ms after incremental layout (was 35 ms),
  scroll frame 6 ms without taffy (was 31 ms). Phone baseline ~2.2 ms CPU per
  frame. The remaining CPU cost of an update is mostly rebuild + flatten +
  paint of the whole tree.

## Questions every design answers

| Question | Why it matters |
|---|---|
| Where does state live? | One root, tree nodes, free-standing objects |
| Who may write it? | Determines whether the flow graph stays a tree |
| How are changes noticed? | Explicit message, equality check, version counter, dependency tracking |
| What reruns on a change? | Whole view, one subtree, one property |
| Where do async results land? | At the owner, or anywhere |
| When does async work stop? | Never, manually, or with its owner |

## Survey

### 1. Single store, unidirectional (Elm, iced, Redux)

One model at the root; events become messages; a pure `update` produces the
next model; `view` derives the whole UI from it.

- **Flow:** the clearest there is. One writer, one direction.
- **Async:** effects as data. iced returns `Task<Msg>` from `update` and
  declares `Subscription`s; Redux uses middleware (thunks, sagas). Results come
  back as messages. Tasks are not tied to any UI node.
- **Performance:** the whole view reruns per message. Mitigated with
  memoization (Elm `Html.Lazy`, iced `lazy`) that skips a subtree when its
  inputs compare equal.
- **Costs:** everything goes through one `Msg` enum, which grows large;
  passing data and messages down through every layer ("prop drilling") and
  mapping child messages back up.

### 2. Component-local state, props down, events up (React hooks, Flutter `StatefulWidget`, SwiftUI `@State`, Compose `remember`)

State is owned by the tree node that declares it. Parents pass data down;
children report events up through callbacks. Changing a node's state reruns
that node's build function and its children.

- **Flow:** a tree. State is readable only below its owner; changed only by
  the owner's handlers.
- **Async:** scoped to the node. React `useEffect` with cleanup, Compose
  `LaunchedEffect`, SwiftUI `.task`. The task's result writes the node's own
  state, and the task is cancelled when the node leaves the tree.
- **Performance:** a change rebuilds the owner's subtree, not the whole UI.
  Frameworks add skipping: React `memo`, Compose skips calls whose parameters
  are stable and unchanged.
- **Costs:** state shared between distant nodes must be lifted to a common
  ancestor, which brings back prop drilling. Identity rules (keys, call
  position) decide whether state survives a rebuild.

### 3. Scoped injection (React Context, Flutter `InheritedWidget`/Provider/Riverpod, SwiftUI `@Environment`, Compose `CompositionLocal`)

An ancestor provides a typed value; any descendant reads it by type without
intermediate layers passing it along. Readers are recorded, so a change
rebuilds only the readers.

- **Flow:** still a tree: visible only below the provider. Writes usually go
  through methods on the provided object, not by replacing it from below.
- **Async:** whatever the provided object does (often an observable store,
  see 4).
- **Performance:** good when readers are few; a frequently changing context
  read high in the tree rebuilds a lot.
- **Costs:** dependencies are implicit (not in function signatures). Fixes
  prop drilling for themes, locale, session, services.

### 4. Observable stores / view models (MobX, Vue `reactive`, SwiftUI `@Observable`, Compose `ViewModel` + `StateFlow`, Flutter `ChangeNotifier`/BLoC)

State lives in objects outside the tree. Views subscribe, explicitly
(streams) or implicitly (reads are tracked during build). A write marks the
subscribed views dirty.

- **Flow:** stores → views, a bipartite graph. Stays clear if stores only
  change through their own methods and stores don't write each other. Becomes
  all-to-all when they do.
- **Async:** natural. The store does the work and publishes results; views
  just react. Store lifetime is independent of the UI, so cancellation is
  manual (Android ties `ViewModel` scopes to screens for this reason).
- **Performance:** rebuilds only subscribed views. Tracking costs a little per
  read and some memory per subscription.
- **Costs:** shared mutable objects. In Rust: `Rc<RefCell<_>>` (borrow panics
  at runtime) or `Arc<Mutex<_>>` across threads.

### 5. Fine-grained signals (Solid, Leptos, Dioxus, Floem, Slint properties, Svelte 5)

State is held in signals; derived values are memos; effects run when inputs
change. Build runs once; afterwards a signal write updates exactly the
properties that read it, with no rebuild.

- **Flow:** a dependency graph of signals, memos and effects. Readable as a
  DAG when effects only touch the outside world; effects that write other
  signals make it hard to follow, the main all-to-all risk.
- **Async:** resources/suspense (Leptos `Resource`, Solid `createResource`):
  a future whose result is a signal. Ownership scopes dispose effects and
  resources with their component.
- **Performance:** the best for small updates: cost ∝ readers of the changed
  signal. Every signal and subscription is a heap node; large static UIs pay
  that up front. Needs a retained widget tree with mutable properties.
- **Costs in Rust:** signals are usually `Copy` handles into an arena so
  closures can capture them; reading a disposed signal is a runtime error.
  Arena types often aren't `Send` without a thread-safe variant.

### 6. Lenses / state projection (Druid, Vizia, Xilem)

One app state, but each widget sees only a slice through a lens or mapping
(Xilem passes `&mut State` through the view tree and narrows it with
`map_state`/`lens`).

- **Flow:** single owner like Elm, without a global message enum: widgets
  mutate their slice directly through `&mut`. The borrow checker enforces a
  single writer at a time.
- **Async:** Xilem has `task`/`worker` views: async work declared in the view
  tree that receives `&mut State` when results arrive, and stops when the view
  is removed.
- **Performance:** Druid compared slices by `Data` (cheap equality) to skip
  updates; Xilem memoizes on inputs. Otherwise a full rebuild, as Elm.
- **Costs:** lens boilerplate; equality requirements on state types.

### 7. Immediate mode (egui, Dear ImGui)

No retained UI state; the UI is code that runs every frame against the app's
data.

- **Flow:** trivial: the app reads and writes its own data directly.
- **Async:** app-managed channels plus `request_repaint`.
- **Performance:** full cost every frame. Conflicts with dfui's retained
  layout, taffy caching and damage tracking.

### 8. Signals/slots, event bus (Qt, GTK, global event emitters)

Any object connects to any other object's events.

- The all-to-all counterexample: flexible, but data flow is only visible at
  runtime. Included as the shape to avoid.

## Data flow: keeping the graph small

The surveyed designs that stay readable share four rules:

1. **One owner per piece of state** (root, a tree node, or a store).
2. **Readers are scoped:** below the owner (2, 3) or declared subscribers (4).
3. **Writes go through the owner:** messages, the owner's handlers, or store
   methods. Never assigned from arbitrary places.
4. **Async results land at the owner,** and work started by a node stops when
   the node goes away.

Together these keep the graph a tree (1, 2, 3, 6) or a two-layer
stores → views graph (4). Signals (5) need a convention that effects don't
write signals; signals/slots (8) break rule 3.

## Performance

What a single state change costs in dfui's pipeline, per design:

| Stage | Elm full rebuild | Elm + memo | Component-granular (2, 3, 4) | Fine-grained (5) |
|---|---|---|---|---|
| Change detection | none (message) | equality of memo inputs | dirty flags / version counters / tracked reads | tracked reads |
| `view` / build | whole tree | whole tree minus memoized subtrees | dirty subtrees | none (property set) |
| `flatten` | whole tree | whole tree | dirty subtrees, rest reused by id | none |
| Layout | incremental already | same | same | same, if the property affects size |
| Paint | whole tree | whole tree | dirty subtrees + damage diff | changed nodes |
| GPU | damage only already | same | same | same |
| Memory overhead | none | cached subtrees | state slots + subscriber lists | a node per signal and subscription |

Observations:

- Layout and GPU are already incremental, so the choice mostly decides
  **build + flatten + paint** cost, the CPU half of an update.
- **Component granularity** matches structures dfui already has: per-node ids,
  `State` slots, and layout reuse by id. Rebuilding only dirty `stateful`
  subtrees is the open "Partial rebuild" item in `TODO.md`.
- **Memoization** is the cheapest change: an equality check on inputs, no new
  tracking. It requires discipline (memo inputs must capture everything the
  subtree reads).
- **Fine-grained** wins on tiny updates to large UIs but needs widgets that
  are retained and mutated in place, replacing the rebuild-and-diff `Element`
  model: effectively a new framework.
- **Tracked reads** (3, 4, 5) cost a small amount on every read and make
  change propagation implicit; explicit invalidation (messages, handle writes,
  version counters) is cheaper and easier to follow but more verbose.
- Paint of untouched subtrees could be skipped by caching each subtree's
  instances in the `Scene`, independently of which state design is chosen.

## Async in each design

| Design | Task owner | Result lands at | Cancellation |
|---|---|---|---|
| 1 Elm | runtime | `update` via `Msg` | manual (ids/tokens) |
| 2 Local state | tree node | the node's state | automatic on removal |
| 3 Injection | provided object | the object | object's lifetime |
| 4 Stores | store | store, then subscribers | manual / store lifetime |
| 5 Signals | ownership scope | a signal | automatic with scope |
| 6 Lenses (Xilem) | task view | `&mut State` | automatic on removal |

All of them need the same runtime primitive from `async-and-state.md`: a
`std::task::Waker` that wakes the event loop, and a local executor on the UI
thread. They differ only in where results go. Tying tasks to tree nodes
(2, 5, 6) gives cancellation for free: dfui already drops `State` for ids
missing after a rebuild, and pending futures can be dropped alongside.

## Candidate designs for dfui

**D1. Elm + tasks + memo.** Keep `App`/`Msg`, add `Task<Msg>` from `update`,
subscriptions, and a `lazy(inputs, || view)` subtree. Smallest change. Still a
global message enum, prop drilling, and full rebuilds outside memoized parts.

**D2. Components.** Grow `stateful` into components: a node owns state,
handles its own events, and spawns tasks whose results update its own state
(`cx.spawn(h, fut, |s, out| ..)`). Data goes down as arguments, events go up
as callbacks or messages. `App` becomes the root component. Add scoped
injection (`provide(value)` / `cx.use::<T>()`) for cross-cutting values,
readable below the provider and written only through the provider's API. A
state change marks its node dirty and only that subtree rebuilds.

**D3. Stores.** Free-standing `Store<T>` handles on the UI thread with a
version counter. Components that read a store during build are recorded; a
write rebuilds them. A `Send` sender lets other threads/runtimes post updates
to a store. Fits app data shared across distant screens; needs D2's dirty
subtrees to pay off.

**D4. Signals.** Retained widgets with signal-bound properties. Best per-update
cost, largest rewrite, and the flow risk from effects.

| | Async | Flow | Update cost | Change size | Web / `!Send` |
|---|---|---|---|---|---|
| D1 | `Task` via `update` | tree, single root | whole view (minus memo) | small | fine |
| D2 | node-scoped, auto-cancel | tree | dirty subtree | medium: dirty-subtree rebuild | fine |
| D3 | store-scoped | stores → views | subscribed subtrees | medium, on top of D2 | `Rc` on UI thread, `Send` sender only |
| D4 | signal resources | DAG (by convention) | changed properties | rewrite | arena per thread |

## Async reactive components

A component that subscribes to a stream (or future) and rebuilds from its
latest value, like Flutter's `StreamBuilder`/`FutureBuilder`. It owns the
subscription, keeps the latest snapshot as local state, rebuilds only its own
subtree per item, and cancels when it leaves the tree.

| | Stream lives in | Item arrives → | Cancelled when | Fit |
|---|---|---|---|---|
| D1 Elm + subscriptions | `App::subscriptions()` | `Msg` → `update` → full rebuild | dropped from the list | poor: not a component, every stream is a global `Msg` variant |
| D2 Components | the node's `State` slot | node state → subtree rebuild | node removed | direct match |
| D3 Stores | a store collecting the stream | store version → readers rebuild | manual / store dropped | good when one stream feeds many widgets |
| D4 Signals | a resource (signal) | bound properties update, no rebuild | scope disposed | good for high-rate streams, at rewrite cost |

Sketch under D2:

```rust
stream_view(
    query.clone(),                    // deps: restart the stream when these change
    move || search(query.clone()),    // factory: runs on mount and on deps change
    |snap: &Snapshot<Vec<Hit>>| match snap {
        Snapshot::Waiting => text("…").into(),
        Snapshot::Data(hits) | Snapshot::Done(hits) => list(hits),
        Snapshot::Error(e) => text(e).into(),
    },
)
```

It is `stateful` with a stream in the slot: on mount `expand` calls the
factory and stores the stream with `Snapshot::Waiting`; the runtime waker
polls it and writes items into the slot, marking that node dirty; when the id
disappears from the tree the existing state cleanup drops the stream, which
cancels the work. A future is a stream of one item.

- **Restart rule.** `view` builds new closures every call, so restarts are
  driven by explicit deps compared by equality (like React effect deps), not
  by stream identity as in Flutter. Otherwise every rebuild resubscribes.
- **Coalescing.** Items arriving faster than frames keep only the latest and
  rebuild at most once per frame.
- **Cost.** One subtree rebuild per frame with a new item: cheap for small
  subtrees (a label, a chart). Large subtrees on fast streams are where D4
  wins.
- **Staging.** Needs only the waker and executor from `async-and-state.md`.
  Without dirty-subtree rebuilds it falls back to a full rebuild, which is
  correct; partial rebuild makes it cheap.
- **Shared streams** (a websocket feeding several screens) go through a D3
  store; each `stream_view` subscribes to the store rather than the source.

### Futures and async closures

Widgets never `.await` in `view`, `paint` or `event`; the runtime holds the
future in the node's slot and polls it. The factory can be an async closure
(stable since Rust 1.85):

```rust
future_view(
    user_id,                                  // deps
    async move || api::user(user_id).await,   // AsyncFnOnce() -> T
    |snap: &Snapshot<Result<User, Error>>| match snap {
        Snapshot::Waiting => spinner(),
        Snapshot::Done(Ok(u)) => text(&u.name).into(),
        Snapshot::Done(Err(e)) => text(e.to_string()).into(),
    },
)
```

- `AsyncFnOnce` fits: `view` makes a fresh closure each call, the runtime
  calls it only on mount or deps change and drops it otherwise; consuming it
  makes the future own its captures, so it's `'static`.
- A `Waiting { previous: Option<&T> }` variant lets a refetch keep showing
  the old value instead of flashing the spinner.
- The future returns **data**, not an `Element`. An element built when the
  future resolves would keep the props, theme and handlers captured at start;
  data is rebuilt by the normal `view` every time. It also leaves no place for
  loading/error UI, and keeping an `Element` alive across rebuilds would be a
  special case in `flatten`.
- Async event handlers (`on_click(async move |cx| ..)`) are the same mechanism
  started from an event: `cx.spawn(handle, fut, |s, out| ..)`.

### In other frameworks

Same task everywhere: load user `id`, show a spinner, then the name.
Snippets from memory; exact names vary by version.

```jsx
// React: effect writing local state; deps array restarts, cleanup cancels
const [user, setUser] = useState(null);
useEffect(() => {
  let live = true;
  fetchUser(id).then(u => live && setUser(u));
  return () => { live = false };
}, [id]);
// React 19: suspend; the nearest <Suspense fallback> shows meanwhile
const user = use(userPromise);
```

```swift
// SwiftUI: modifier writing the view's own state, cancelled on disappear/id change
@State var user: User?
Group { if let user { Text(user.name) } else { ProgressView() } }
    .task(id: id) { user = try? await api.user(id) }
```

```kotlin
// Compose: state produced by a coroutine keyed by id; Flows via collectAsState
val user by produceState<User?>(initialValue = null, id) { value = api.user(id) }
```

```rust
// Leptos / Solid: a resource is a signal fed by a future, under a Suspense boundary
let user = Resource::new(move || id.get(), |id| async move { api::user(id).await });
view! { <Suspense fallback=|| view!{ <Spinner/> }>
          {move || user.get().map(|u| view!{ <p>{u.name}</p> })}
        </Suspense> }

// Dioxus: hook returning Option
let user = use_resource(move || async move { api::user(id()).await });

// iced: effects as messages, no widget involved
Msg::Open(id) => Task::perform(api::user(id), Msg::Loaded),

// egui: poll a promise every frame
match promise.ready() { Some(u) => ui.label(&u.name), None => ui.spinner() };
```

```js
// Query caches (TanStack Query, SWR, Riverpod FutureProvider): shared by key
const { data, isPending } = useQuery({ queryKey: ['user', id], queryFn: () => fetchUser(id) });
```

| Axis | Variants |
|---|---|
| Where the result lives | wrapper widget's slot (Flutter builders, Dioxus) · the component's existing state (SwiftUI `.task`, Compose, React effects) · a signal (Solid, Leptos) · app model (iced) · shared cache (TanStack, Riverpod) |
| How loading is shown | branch on a value (`Snapshot`, `Option`, `isPending`) · Suspense: the reader suspends and the nearest ancestor boundary shows a fallback (React, Solid, Leptos) |
| Restart | deps/key (React, SwiftUI `id:`, Compose keys, Leptos source, query key) · object identity (Flutter) · explicit message (iced) |
| Cancellation | with the node (SwiftUI, Compose, Leptos, Dioxus) · manual cleanup (React) · none (iced) |
| Sharing | per component (most) · deduplicated by key across components (query caches) |

### Layering for dfui

1. **Primitive: a task modifier writing the owner's state** (SwiftUI `.task`,
   Compose `produceState`). `stateful` already owns state, and results landing
   at the owner is data-flow rule 4.
   ```rust
   stateful(|s: &Profile, h| match &s.user { Some(u) => text(&u.name).into(), None => spinner() })
       .task(id, async move || api::user(id).await, |s: &mut Profile, u| s.user = Some(u))
   ```
2. **Sugar: builders** (`future_view`, `stream_view`): the primitive with the
   state fixed to `Snapshot<T>`.
3. **Later: Suspense boundaries.** One fallback for a whole screen of nested
   loads; needs a child to report "pending" to an ancestor during `flatten`.
4. **Later: query cache.** Dedup and sharing by key across screens; a D3 store
   library on top of 1, not core.

## Leaning

D2 looks like the best fit, with D1's `lazy` as a cheap escape hatch and D3 as
a possible later addition:

- It extends what exists (`stateful`, `Handle`, per-id `State`, layout reuse)
  rather than replacing it.
- Async results land at the owning node and stop with it, which covers most UI
  async (loading a screen's data, debounced search) without global messages,
  and makes future/stream-driven components (`.task`, `future_view`,
  `stream_view`) a thin layer on top.
- Ownership stays a tree, with injection for the few app-wide values.
- Update cost becomes proportional to the dirty subtree, which is the open
  partial-rebuild work anyway.

## Decision

Implemented (see `architecture.md`, "Widgets and state" and "Async"):

- **Messages removed.** `App` is just `view` + `background`; UI state lives in
  `stateful` widgets, app-wide UI state in a root `stateful`.
- **Providers** inject external state: `provide(value, child)`, read with
  `Scope::get::<T>()`. Values are whatever the app owns (clients, channels,
  `Rc`/`Arc` stores), so heterogeneous external state needs no framework type.
- **Builders** are the async entry point: `future_builder` / `stream_builder`
  take a factory that reads providers and returns any std `Future` or
  `futures_core::Stream`. Streams collect into `S: Extend<Item>`, so the
  Flutter "latest value" and "collected list" cases share one API.
- **Executor** on the UI thread, woken through a winit user event; tasks keyed
  by node, restarted on deps change, cancelled with their node. No runtime
  dependency, no `Send` bound; works the same on web.
- The demo's Async tab: a provided job runner spawns a thread that sleeps and
  sends entries over a channel; a `stream_builder` collects them into a
  scrolling list.

Not yet: the `.task` modifier on `stateful`, async event handlers, sync
provider reads in `stateful` builds, Suspense boundaries, dirty-subtree
rebuilds.

## Open questions

- Component identity: is position + `key` enough when components own
  long-lived async work, or do moved subtrees need explicit identity?
- Can a parent read a child's state, or only receive its events? (React: no;
  Flutter: via `GlobalKey`.)
- Do messages to `App` remain, or does `App` become just the root component?
- Injection change notification: rebuild every reader, or compare by value?
- Re-entrancy: a task result arriving mid-dispatch is queued and applied with
  the other updates after dispatch, as handle updates are today.

## Evaluating

Prototype D1 (`lazy`) and D2 (dirty subtrees) against the 1000-card bench:
update one card from an async source and compare build + flatten + paint
time, and write the same small app (list + detail screen + background fetch)
in each to compare how readable the data flow is.
