# dfui roadmap

Baseline (Pixel 10a, Mali-G715, release build): bench screen of 64dp text at
60 fps, ~5.1 ms GPU / ~2.2 ms CPU per frame; normal demo screen ~4.5 ms GPU.
At 120 Hz (full redraws forced): bench ~3.2 ms GPU / ~1.8 ms CPU, demo
screen ~2.6 ms GPU; the GPU likely clocks higher at 120 Hz.

## Next

- [x] Text input + keyboard focus: single-line `text_input`, tap to focus,
      Android soft keyboard, caret.
- [x] Text input selection (shift+arrows/click, drag, select all), clipboard
      (arboard on desktop, in-app on Android), caret blink.
- [ ] Text input follow-ups: desktop IME composition, multi-line,
      keyboard-aware insets on Android, Android system clipboard and touch
      selection handles.
- [x] Rejected after measuring: curves inline in each band list, to drop one
      dependent load per curve. Text wall 113 → 109 µs on desktop, 3.41 ms
      unchanged on the Pixel 10a, for ASCII outlines growing 89 → 253 KB.
- [x] Status bar icon color: dark icons over a light `App::background`
      (API 30+).

## Findings from a real app

From building the movies-thing music player (dfui on Android and Linux: a
custom card-stack widget, background downloads, a playback service).

Android lifecycle and platform:

- [x] Leave the run loop on `MainEvent::Destroy`: Android runs on
      android-activity directly instead of winit, whose loop ignores
      Destroy and can't be created twice in a process. `run_android`
      returns on Destroy and runs again for a recreated activity.
- [ ] Packaging beyond cargo-apk: services and app Java classes can't be
      declared, so the app builds its APK by hand (javac, d8, aapt2
      compile/link, zipalign, apksigner). Ship that as a script/template, or
      fold it into the cargo-apk replacement (Tooling). Resources do work:
      busapp ships an adaptive launcher icon with `resources = "res"` and
      `icon = "@mipmap/ic_launcher"`; worth a line in the README's Android
      section.
- [x] JNI helper: `dfui::android::with_activity`; its module docs say how
      to reach app classes from native threads.
- [x] Logging: `log` + `android_logger`, documented in the README.

State and async:

- [x] A notify handle any thread can trigger to rebuild:
      `Notifier::notify()`, watched by `widgets::watch(&notifier, || view)`.
- [ ] Keep-alive / offstage children (an `IndexedStack`): state is keyed by
      tree position, so switching tabs drops the hidden page's state (e.g.
      its scroll offset).
- [ ] Frequent background updates (download progress at 2 Hz) rebuild the
      whole tree; with `list` only the rows near the viewport are built.
      See "Partial rebuild (CPU)".

Layout:

- [x] `min_width`/`min_height` modifiers and `fill()` (grow into the space
      left, zero basis and minimum) for flex boxes around a `scroll`, which
      otherwise grows to its whole list; `scroll`'s docs say so.
- [x] Crash when a container that held a layer (dropdown menu) loses all
      its children: taffy 0.15 keeps the layer in the leaf's hoisted list
      and panics on it when rounding. The layout gives such a container a
      fresh taffy node. Worth reporting upstream.

Custom widgets:

- [x] Public kinetic drag (`dfui::kinetic::Drag`: slop, release velocity,
      fling decay); `scroll` uses it.
- [x] `PaintCx::with_clip(rect, |cx| ..)`.
- [x] `PaintCx::measure` for text width/height in `paint`.

Text:

- [x] Ellipsis: `text(..).max_lines(n)`, and `Text::truncate` /
      `PaintCx::truncate` for custom widgets.
- [x] `Font::BOLD` (bundled DejaVu Sans Bold).
- [x] `text_input(..).password()`: a dot per char, no copy or cut.
- [x] Input type hints for the Android keyboard: `text_input(..).keyboard(..)`
      (text, number, email, password); password fields turn off learning.

Widgets:

- [x] Icon button: `icon_button(&shape)`, borderless, shrinks on press and
      springs back past its size; `icon(&shape)` without the button. Shapes
      (`Shape`) are stored once like glyphs and drawn into any rect.
- [x] `tabs` slots sized by their labels (plus a share of the spare width),
      so underline and taps match what's drawn; the selected label morphs
      to bold. Tab bars wider than the window still overflow (clipped).
- [ ] `tabs`/`segmented` styling: colors follow the `Theme`; label font and
      size are still fixed.
- [x] Progress: `progress(v)`, `stacked_progress` (several fractions in
      one bar, smaller on top), `wait_bar()` and `spinner()` for waits of
      unknown length.
- [x] Virtualized list: `list(model, row)` over a `ListModel` builds only
      the rows near the viewport, keeps the first visible row in place by
      key, and slides rows in, out and to new places (`docs/data-models.md`).
- [x] `table` (sortable columns) and `tree_view` (`TreeModel`, flattened
      into rows) on top of it.
- [ ] Data model follow-ups: selection by key, filter adapters, lazy tree
      children, paged remote data, horizontal virtualization for wide
      tables.
- [ ] List row: title, subtitle, trailing actions.
- [ ] Image fit modes and a placeholder (also under Image follow-ups);
      apps downscale covers themselves for lack of mipmaps.
- [x] Stroked SVG icons (Feather) as vectors: the music app parses them
      with usvg, turns strokes into fills and draws them as `Shape`s; dfui
      itself stays free of an SVG parser.

## Findings from a second app

From building bbteam (a Blood Bowl team manager on Android and Linux: card
deck, forms, a two-column game log, bottom sheets, dialogs, an image picker).

Bugs found and fixed:

- [x] Soft keyboard never opened: `set_ime` called `show_soft_input` from
      the render thread and Android cancelled every request
      (`onCancelled at PHASE_CLIENT_APPLY_ANIMATION`). It now runs on the
      Java UI thread, and the request is explicit (implicit ones are
      ignored once the user has hidden the keyboard).
- [x] Tapping the already-focused input didn't bring back a keyboard the
      user dismissed: `set_focus` only asked for it on a focus change.
- [x] Letters typed on some physical keyboards (the emulator's host
      keyboard) were dropped: their key character map gives no character,
      so only Backspace and arrows arrived. Falls back to the virtual
      keyboard's map.
- [x] `overlays()` wrapped its content in `grow(1.0)` instead of `fill()`,
      so no `scroll` inside an overlays host ever scrolled: it grew to its
      whole content and pushed the widgets after it off screen.

Android:

- [x] Editor view: `dev.dfui.EditorView` gives the keyboard a real
      `InputConnection` (`inputType` text, Gboard's toolbar shows). Its dex is
      embedded and loaded at runtime (`scripts/android-dex.sh`), so apps need
      no Java or packaging. `commitText` and deletes reach `text_input`.
- [ ] Editor view follow-ups: send the text and selection to the `InputConnection`, so
      predictions and autocorrect see context (`getTextBeforeCursor` is empty);
      verify composing on a device with the on-screen keyboard.
- [ ] Image picking needs app Java: an activity method that starts
      `ACTION_GET_CONTENT`, copies the result to app storage and calls a
      native method, which only resolves if the activity runs
      `System.loadLibrary` in a static initializer (NativeActivity loads the
      library itself). Offer `dfui::android::pick_image` or document the
      pattern; on desktop the app shells out to `zenity`.

Layout and scrolling:

- [x] Programmatic scrolling: `scroll(..).stick_to_end()` follows a log's
      newest line and `start_at(fraction)` jumps. A controller that scrolls
      to a given widget is still open if an app needs one.
- [x] A `scroll` with `shrink(1.0)` shrinks below its content on taffy 0.15
      without `min_height(0.0)`. The trap left is a column between the window
      and the scroll without `.fill()`: it grows to the content, as `scroll`'s
      docs say.
- [x] Equal-width buttons in a row need `.width(0.0).grow(1.0)`; `fill()`
      on each already does it, and its docs say so.
- [ ] Height-for-width leaves: a card whose height follows its width needed
      a custom `Widget::measure`; an aspect/height-for-width modifier would
      cover it.

Widgets:

- [x] Disabled buttons: `button(..).enabled(false)` dims and ignores input.
      Same for `icon_button`.
- [ ] `icon_button`'s 10px padding makes grids of icons sparse (16 roster
      slots wrapped onto three rows); and an inert slot drawn with `icon`
      is a different size from the buttons beside it. Expose the padding
      in the docs, or an inert `icon_button`.
- [x] Confirm dialog: `ov.confirm(cx, title, body, ok, on_ok)`.
- [x] Back handling for app screens: a public `on_back(f)`. A navigation
      stack is still open if a third app wants one.
- [x] `dropdown` with many options (30 races) runs off the window: the menu
      scrolls past a maximum height and opens at the selected option.
- [x] Chips: `chip(label)` and a tappable `choice_chip(label, selected)`.
- [x] Badge: `badge(child, label)`, a count over a corner of another widget.
- [ ] Numeric text input: `Keyboard::Number` shows digits, but there's no
      filtering or maximum length (a one-digit field keeps only the last digit
      typed, by hand).
- [x] `stateful_with(init, build)`: state from an initial value, for forms
      over existing data; it needn't be `Default`.
- [ ] The card stack (fold/unfold, kinetic scroll, shadows on the card
      above) is now copied in two apps: worth a dfui widget with a
      card-painting callback.
- [x] Color helpers: `Color::mix`, `lighten` and `darken`.

Theming:

- [x] `dialog`, `bottom_sheet` and `dropdown` are white with dark text on a
      dark app: they now follow the `Theme` (`docs/theming.md`).

## Findings from a third app

From building busapp (Innsbruck bus departures and a trip planner on
Android: stop search, live departures, line badges in IVB colors, a
force-laid-out line network, per-line stop lists and an OSM map).

Bugs found and fixed:

- [x] Crash leaving a page whose dropdown had been open (taffy 0.15 hoisted
      list, see Layout above). Not yet reported to taffy.

Patterns the app hand-rolled that belong in dfui:

- [x] Page scaffold: `page(title).back(f)` / `.menu(f)` puts an arrow
      (also run by Escape and Android back) or a menu button before the
      title, then the body; `heading` for the title text. `dfui::icons` has
      `menu`, `back` and `close`, stroked in code. `on_back`/`on_key` take no
      layout space, so they no longer add a gap where they're placed.
- [x] Polled sources: `polled(fetch, again)` streams a fetch repeated after
      the delay `again` picks per result; `blocking(work)` runs `work` on a
      thread with a `Wanted` check it can poll once nobody waits (not on
      the web); `Latest<T>` keeps a stream's newest item.
- [x] Results that outlive their node: `cached(&cache, key, fetch, build)`
      builds from a `Cache<K, V>` hit, or fetches once and stores the value
      (failures aren't stored).
- [x] Polyline strokes in core: `path::stroke(points, Stroke)` (polygons,
      for callers that clip them) and `Path::stroke`; square or round caps,
      bevelled bends. `dfui-map` uses it. The spinner keeps its own: a
      curved spiral of varying width.
- [x] Custom widget ergonomics: `EventCx::with_state` and
      `PaintCx::with_state` lend the state and the context together; a
      public `OnChange<T>` replaces the four private copies, for custom
      widgets that report a value. A closure stored in a variable before
      `future_builder`/`cached` still needs `|_: Scope|` spelled out.
- [ ] Public headless test driver: the dropdown crash was reproduced by
      tapping through real screens with `Bench` (`event`, `advance`,
      `shot`), but it is `#[doc(hidden)]`, has no system-bar insets (taps
      needed a −24 px offset against the device) and takes raw pointer
      events. A `tap(x, y)` with device-like insets, together with the
      `snapshot` item under Testing.
- [ ] Label placement: busiest first, right of the point or else left,
      dropped if it would overlap a placed one, so more show when zoomed
      in. The network view does it; `dfui-map`'s street labels face the
      same problem.
- [ ] Android location and permissions: busapp's `gps.rs` asks for the
      permission and keeps the providers updating through a `PendingIntent`
      nobody receives (no Java class), reading the newest fix once a
      second. A `dfui::android` helper would save the JNI.
- [ ] arm64 builds crash on x86_64 emulators when typing: the
      `EditorView` natives are registered by raw pointer and, under
      Berberis translation, ART calls the arm64 code as x86
      (SIGSEGV in `commitText`). Investigate; at least tell apps in the
      README's Android section to build `x86_64-linux-android` for the
      emulator.
- [ ] Release template: names and busapp share a workflow (tag → tests →
      signed `cargo apk build --release` → GitHub release, keystore in
      secrets). Document it in dfui's README or ship it as a template;
      covers "Release signing config" under Tooling.

Layout and widgets:

- [ ] `button().flat()` only drops the shadow; the background stays
      `Role::Accent` and the child is centered. A plain list row took
      `.flat().color(Color::WHITE).justify(JustifyContent::START)`. A
      borderless row style, or at least docs saying what `flat` keeps.
- [ ] Text in a `row` shrinks and wraps by default: a "5 min" label beside a
      long name broke onto two lines until `.shrink(0.0)`. Consider labels
      not shrinking below their width by default, or document it.
- [ ] A fixed `.width()` on a `container` doesn't hold when the content is
      wider (min-content wins), so badges of "3" and "STB" differ in width.
      Flex behavior, but surprising; document next to `width`.
- [ ] Readable text on an arbitrary background: the app computes brightness
      to pick dark or white text on each line color. `Role::OnAccent`
      covers theme colors only; a small `Color` helper would cover the rest.

Testing:

- [ ] `snapshot()` renders one frame, so `future_builder`/`stream_builder`
      never resolve in it and real screens can't be checked headless. Poll
      ready tasks before rendering (or take a frame count) so an app with
      stubbed sources can snapshot its actual screens.

## From the code review
- [x] Shape storage: equal shapes share one copy, and shapes not drawn for 600
      painted frames give their space back.
- [ ] Android editor: `deleteSurroundingText` ignores text after the caret; no
      `setComposingRegion` or `setSelection`.
- [x] Late `with_activity` calls on a destroyed activity are safe (`AndroidApp`
      keeps its reference alive); the editor view no longer outlives the activity.
- [x] Sources faster than a rebuild hung the UI: `drive` re-polls only new
      tasks, `stream_builder` drains at most 1024 items per poll, and wakes
      are handled in `about_to_wait`.
- [x] A touch cancelled by the system left a tooltip stuck: a cancel now ends
      with `Leave`, like a lift.
- [ ] Tab bars can't scroll when their labels don't fit (`segmented.rs`).
- [ ] Long functions: `Runtime::dispatch`, `Renderer::new`, `Renderer::draw`,
      `layout::measure`, `text_input::event`.
- [ ] List ghosts are recognized by `position == Absolute` (`Slot::inert`);
      use an explicit flag.
- [ ] Test hooks in production code: `ime_requests` on `Runtime` and the
      settable clock. `Bench`, `GpuBench` and `snapshot` are `#[doc(hidden)]`.
- [x] Public API: `Event`, `Key` and `Keyboard` are `#[non_exhaustive]`.
      `taffy` stays re-exported (`Widget` speaks its `Style`), so a `taffy`
      upgrade is a dfui breaking change.
- [x] `Shape::from_bytes` returns `None` on a bad or truncated file.
- [x] Crate metadata, MSRV (1.90, from wgpu) and an `include` list for
      `cargo package`.
- [x] Startup failures exit with a message, invalid fonts fall back to the
      default, and `Bitmap`'s size check can't overflow.
- [ ] Duplicated cubic-to-quadratic splitting in `outline.rs` and `svg2path`.
- [ ] Tests: goldens only run on the reference machine (a tolerance or a pinned
      software GPU would let CI run them); pattern tests use fixed coordinates.
- [x] Benches for `list`, `table` and typing (`cpu`: `list/*`, `table/*`,
      `typing/*`).
- [ ] Run the `list`, `table` and typing benches on the phone and compare
      them against the earlier baseline.
- [ ] Docs: 178 undocumented public items; `site/pages/concepts.md` is a stub.

## Design studies (need discussion before code)

- [x] Global state management: messages removed; `provide` injects
      values and services, `stateful` owns UI state, builders read async
      sources (`docs/state-management.md`, "Decision").
- [x] Web port: WebGPU in a canvas, GPU setup polled as a future,
      `web_time`, in-app clipboard, `dfui::sleep` on event-loop timers. Docs
      site with a live example per widget, deployed to GitHub Pages
      (`site/`, `scripts/site.sh`, `.github/workflows/pages.yml`).
- [ ] Web follow-ups: each example is ~2.2 MB of wasm before `wasm-opt`
      (CI doesn't run it yet); the demo's job runner uses OS threads, so the
      demo itself isn't on the web; system clipboard and IME on the web;
      write the site's introduction and concepts pages.
- [x] Partial repaint (GPU): scissored retained canvas, per-node damage
      from a scene diff, idle frames skipped (`docs/rendering.md`). Still to
      measure on the phone against the Bench baseline.
- [ ] Partial rebuild (CPU): every change still runs the whole `App::view`,
      flatten and a full paint. Layout is incremental: the taffy tree is kept
      and only changed nodes are dirtied (1000-card edit frame 35 → 11 ms),
      and redraws without a rebuild skip taffy (scroll frame 31 → 6 ms).
      Open questions: rebuild only dirty `stateful` subtrees; skip off-screen
      scroll content; how this interacts with global state change
      notification.

## Framework

- [x] Paint animations: `Animated<T>` eased by `PaintCx::now`, looping via
      `request_frame` while running; button hover/press fade.
- [x] Path morphing (`anim::Morph`): glyph or polygon outlines, drawn by the
      glyph renderer; demo morphs rect ↔ hexagon and D ↔ O.
- [x] Weight morphs pair corner to corner: segment counts differ between
      regular and bold (n: 16 vs 19), and pairing by index slid stem
      corners along their edges, notching them halfway.
- [ ] Morph any glyph into any other: corner counts differ, so pairs fall
      back to index pairing; a curve-matching pass (Sederberg & Greenwood
      1992, dynamic programming) would replace it.
- [ ] Move the hybrid morph from `demo/examples/glyph_lab.rs` into
      `anim::Morph`: dense DP with corners pinned, cutting the exact curves.
      It tears about half as often and keeps corners. Skeleton strokes are
      the candidate for topology changes and a pen animation.
- [x] Masked paths (`PaintCx::path_masked`): the pen in `glyph_lab` reveals
      the real outline, so its edges match the rendered glyph.
- [ ] Tune the pen route in `glyph_lab` (`plan`): E, G, J, Q, g, 9 start
      away from where a hand would.
- [ ] Morphs across topology changes (O → C): an unpaired hole shrinks to
      its centre but can leave the outer contour on the way, showing as fill.
- [ ] Layout animations (size/position): need a rebuild or relayout per
      frame while running. List inserts, removals and moves animate
      without one (FLIP offsets).
- [x] Timed wakeups (`PaintCx::request_frame_at`): the caret blink sleeps
      between toggles instead of painting every frame.
- [x] Theming: `Theme` of named colors, from `App::theme` or `provide`
      (`docs/theming.md`). Sizes and fonts are still per-widget.
- [x] Images: app-decoded `Bitmap`s, a texture and draw each; demo
      gallery downloads from placehold.co.
- [ ] Image follow-ups: mipmaps, fit modes (contain/cover), a decode
      thread pool.
- [x] Async: `future_builder`/`stream_builder` polled on the UI thread,
      woken through the event loop, cancelled with their node.
- [ ] Async follow-ups: task modifier on `stateful` (results into its own
      state), async event handlers (`cx.spawn`), sync provider reads in
      `stateful` builds, Suspense-style boundaries.
- [x] Pinch: two fingers drive one pointer at their midpoint plus
      `Event::Pinch`; ctrl+wheel and touchpad pinch on desktop.
- [ ] Multi-touch follow-ups: a two-finger touch held still and released
      still taps what's under it unless a widget captures; no rotation.
- [ ] Maps experiment (`map/`, `dfui-map`, on crates.io): OSM geometry cut into
      quadtree tiles of `Shape`s; numbers and limits in `map/README.md`.
      Runs on the phone (`osm_android`, the bus app). Maps save to bytes,
      keep lines by their middle and carry levels of detail; tiles stream
      in under a per-frame budget, coarser levels standing in. 8–12 ms GPU
      frames on the Pixel. Next: golden tests, labels stored as text,
      fling after a pan.
- [x] Full-screen layers: `screen` and `backdrop` cover the system bars
      too, content padding itself by `insets()`; the bus app's map runs
      under translucent bars.
- [x] Kinetic scrolling: a released drag flings with exponential friction,
      stopping at the edges; re-placed per frame via `request_placement`.
- [ ] Horizontal scroll; overscroll bounce/glow.
- [ ] Widget library: checkbox.
- [x] Layers: widgets that float above the tree outside its layout
      (`Widget::layer`); `dialog`, `drawer`, `bottom_sheet`, `snackbar`,
      `tooltip`, `dropdown`.
- [x] Overlay host (`overlays`, `Handle<Overlays>::show`, `Shown::close`):
      slide in and out, removal after the exit animation on a `sleep` timer.
- [x] Escape (Android back) dismisses the topmost overlay
      (`Event::Back`); snackbars shown through `Overlays` queue, one at a
      time.
- [x] Layer follow-ups: the dropdown's menu slides open and shut; closing
      a snackbar pulls the queue forward; popups and tooltips flip to their
      anchor's other side (or shift sideways) to stay in the window; tooltips
      show on a long press and linger; anchored popups hide with their
      anchor when it is scrolled away.
- [x] Slides clear the window by the content's own distance (bottom
      sheets and snackbars were off-screen for most of their motion), and a
      flipped dropdown menu unrolls out of its field (`Widget::scroll` gets a
      `Placement`: clip, resting box, anchor).
- [x] `bottom_sheet` runs past the window's bottom edge, so only its top
      corners show rounded.

## Text

- [x] Multiple fonts: app-supplied `Font` bytes (bold/italic are just more
      fonts), glyphs keyed by (font, glyph id), fallback to the default font.
- [x] Shaping with harfrust (read-fonts replaces ttf-parser): GPOS kerning,
      ligatures, marks, Arabic joining. Each word is shaped on its own.
- [ ] Bidi (unicode-bidi): mixed-direction lines come out in logical order.
- [x] Caret moves, deletes and snaps by grapheme in `text_input`
      (unicode-segmentation), never between a base and its mark or inside an
      emoji; inside a ligature it sits at an even split of the glyph.
- [ ] Fallback chains beyond the default font (emoji, CJK), system fonts.
- [ ] Line breaking beyond spaces (long words/URLs).
- [ ] Adaptive cubic→quadratic split for CFF fonts (fixed 4-way split today).
- [ ] Test with variable fonts / overlapping contours.
- [ ] True signed distance (cubic solve) if text outlines/glow/blurred shadows
      are needed; current shader computes coverage only.

## Performance

CPU baseline (`cargo bench`, desktop): one frame of rebuild + layout + paint
+ diff costs ~265 µs for 10 cards and scales linearly (~32 µs/card, 32 ms
for 1000). Laying out a 300-char paragraph: ~34 µs uncached (shaped), ~0.1 µs cached.

2026-11-08, `be2b81e` against `bedc189` on desktop (alternating runs): no
regression; every CPU frame bench is within the run-to-run noise (up to ~10%
at 1000 cards) and `gpu/cards_10` and `gpu/text_wall` are unchanged. Not yet
repeated on the phone.

2026-11-26, desktop: a virtualized list still costs O(n) per frame.
`list/frame` 263 µs at 1000 rows, 9.7 ms at 100 000; `list/scroll` 131 µs →
950 µs. `table/frame` 400 µs → 10.2 ms, 439 µs → 12.8 ms sorted (each
rebuild sorts the whole model). A keystroke (`typing/cards`) costs 16 µs
alone and 872 µs above 100 cards, since every change rebuilds the whole view.

- [x] Cheaper list rebuilds: hashing every key was ~90% of a 100 000-row
      rebuild, so the key index is reused while the keys are unchanged and
      `diff` only visits on-screen rows. `list/frame` 8.25 → 0.55 ms,
      `list/scroll` 699 → 120 µs, `table/frame` 7.86 → 0.72 ms at 100 000.
- [ ] List frames still O(n): reading every key to spot a change and the
      height prefix sum (~0.5 ms at 100 000). Needs a change version on
      `ListModel` and summed heights to go away.
- [ ] `table` sorts the whole model on every rebuild (2.7 ms sorted vs
      0.72 ms unsorted at 100 000): keep the order until the sort or keys change.

- [x] Cache text layouts per (text, size, width), evicted after an unused
      frame. Cut frame CPU ~82%.
- [x] Cheaper layout misses: shaped words are cached at size 1, so changing
      text only shapes what's new (changing label 7.9 → 2.2 µs desktop, 6.7 → 2.5 µs Pixel 10a).
- [ ] Word cache limits: it caps entries (4096) but not bytes, so skip very
      long words; add a test for the clear when full.
- [x] Run the benches on the phone (`scripts/android-bench.sh`): partial
      frame ~44× cheaper than full on Mali. Still unmeasured in the real
      app: canvas copy and `LoadOp::Load` bandwidth.
- [x] Cheaper shadows: drawn as a ring around the opaque shape on top of
      them (`gpu/cards_10` 156 → 133 µs desktop; no measurable change on
      the Pixel 10a in A/B runs).
- [x] Paint culling: nodes whose box (plus a 48px overdraw margin for
      shadows) is outside their clip aren't painted. 1000 cards: edit frame
      12.1 → 8.5 ms, scroll frame 4.6 → 1.2 ms (desktop).
- [x] Upload only the outline spans that changed, not the whole store.
- [x] Flatten once per rebuild: ids cached, other passes re-walk the tree
      (`docs/tree.md`). Pointer move −57%, scroll frame −21% at 1000 cards.
- [x] Pointer events visit only the nodes under the pointer, the ones it
      just left or pressed, and layers; the walk skips other subtrees.
      Pointer move at 1000 cards 282 → 4.3 µs (a linear scan of the boxes
      is what's left).
- [x] Profile 120 Hz: Pixel 10a at 120 Hz with every frame forced to a full
      redraw holds 120 fps; Bench screen ~3.2 ms GPU / ~1.8 ms CPU, main
      screen ~2.6 ms GPU / ~2.3 ms CPU, against an 8.3 ms budget.

- [x] The present pass (canvas → screen, since the HDR work) against the old
      canvas copy, Pixel 10a at 1080×2400, GPU timestamps around each: copy
      148–165 µs, present pass 187 µs from the half-float canvas and the same
      from an 8-bit one. So ~30 µs more a frame, and the half floats are free.
- [x] One submit per frame: draw and present go in one encoder. The timer
      had spanned two submits and the idle gap between them, which on Android
      is large: `gpu/text_wall_partial` read 550–590 µs, now 237 µs, the same
      as before the HDR work once its untimed copy is counted (65–73 µs +
      ~155 µs). Full frames unchanged: `cards_10` ~2.3 ms, `text_wall` ~3.7 ms.

GPU optimization opportunities, from reading `shader.wgsl` (unmeasured;
time each with `GpuBench` and `scripts/android-bench.sh` against the
baseline above, and use `DFUI_SHOW_COST` to see what a screen is bound by):

- [x] Rejected after measuring: 24 vertices per instance. Cutting holed
      shadows into up to four strip instances on the CPU (`clip` per strip)
      let every instance draw 6 vertices, a 4× smaller vertex load. Pixel 10a
      A/B, alternating binaries: no difference outside the ±10% run-to-run
      noise; these screens are fragment- and bandwidth-bound.
- [ ] Fewer varyings per vertex (~28 floats today, written to memory per
      vertex on Mali). Less than it looked: `border_color` carries a glyph
      mask's band header (integers) and the canvas is now half float, so
      only `rect` → `vec2` and `color` → two `pack2x16float` words remain,
      28 → 24 floats. Vertex-side work measured as noise above; low value.
- [ ] Guard `cost += 1u` in `ray_coverage` with `if show_cost`. Likely a
      no-op: `show_cost` is an override constant, so the pipeline is
      specialized and `cost` is dead code when it's off.
- [ ] Shape anti-aliasing without derivatives: `length(dpdx(d), dpdy(d))`
      is about one physical pixel in logical units near an edge; pass the
      scale in `Globals` instead. `fwidth(d)` would be shader-only but
      softer on diagonals. Marginal.
- [ ] Glyph/path coverage (`ray_coverage`, two rays per pixel) has no
      shader-only win left: it already stops at curves behind the pixel.
      Further gains need data changes (half-precision curves, more bands
      for large paths such as map tiles, which are likely loop-bound).

## Tooling

- [x] One file per widget example: `site/examples/<name>.rs` is the site's
      code and wasm and the widget's doc example; doc tests compile it and
      the README's snippets in CI.
- [x] Golden-image tests (`tests/golden.rs`): widgets rendered offscreen
      on a virtual clock (`Bench::advance`/`settle`), animations as frame
      strips, compared against PNGs from the reference machine.
      `DFUI_BLESS=1 cargo test --test golden` rewrites them.
- [x] `tools/svg2path`: converts SVG icons, strokes included, into a compact
      asset that `Shape::from_bytes` loads, so apps needn't parse SVG at
      runtime.
- [x] `image` is a dev-dependency only: `snapshot` returns plain `Pixels`.
- [x] Integration tests of usual app patterns, driven by input events
      (`tests/patterns.rs`).
- [ ] Fold the demo's `#[ignore]` snapshot tests into the goldens.
- [ ] Release signing config for real distribution: apps sign in CI with
      `CARGO_APK_RELEASE_KEYSTORE` from secrets (see "Release template"
      under the third app's findings).
- [ ] cargo-apk is deprecated and capped at NDK platform 35; evaluate
      xbuild or cargo-ndk + Gradle.
