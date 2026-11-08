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
- [ ] Faster outline text: store curves inline in each band list (drop the
      index indirection, one fewer dependent load per curve). Re-measure with
      the Bench screen.
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
- [ ] Packaging beyond cargo-apk: services, resources (launcher icon) and
      app Java classes can't be declared, so the app builds its APK by hand
      (javac, d8, aapt2 compile/link, zipalign, apksigner). Ship that as a
      script/template, or fold it into the cargo-apk replacement (Tooling).
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
- [ ] Input type hints for the Android keyboard (password, number, email):
      android-activity's `set_ime_editor_info` is a no-op on NativeActivity,
      which has no editor view to carry an `inputType`.

Widgets:

- [x] Icon button: `icon_button(&shape)`, borderless, shrinks on press and
      springs back past its size; `icon(&shape)` without the button. Shapes
      (`Shape`) are stored once like glyphs and drawn into any rect.
- [x] `tabs` slots sized by their labels (plus a share of the spare width),
      so underline and taps match what's drawn; the selected label morphs
      to bold. Tab bars wider than the window still overflow (clipped).
- [ ] `tabs`/`segmented` styling: label font, size and colors, and the
      divider color are fixed for a light theme (see Theming).
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
      tables; ghosts of removed rows still take taps.
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

- [ ] The keyboard connects to the decor view with `inputType=0`: Gboard in
      hardware-keyboard mode shows its floating toolbar for stock apps but
      nothing for a dfui app. An editor view (`onCheckIsTextEditor`,
      `onCreateInputConnection`) would fix this and the input type hints
      (see Text); check where IME `commitText` then goes.
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
- [ ] Confirm dialog: every destructive action builds the same dialog
      (title, body, Cancel, OK). A `confirm(ov, title, body, ok, on_ok)`.
- [x] Back handling for app screens: a public `on_back(f)`. A navigation
      stack is still open if a third app wants one.
- [x] `dropdown` with many options (30 races) runs off the window: the menu
      scrolls past a maximum height and opens at the selected option.
- [ ] Chips: skill, injury and status chips are hand-made containers
      (text, background, radius, padding). A `chip(label)` widget, and a
      tappable `choice_chip`.
- [ ] Badge: a count on a corner of another widget (SPP on a number
      circle) is painted by hand. A `badge(child, label)`.
- [ ] Numeric text input: no number mode or maximum length (a one-digit
      field keeps only the last digit typed, by hand). Ties into the input
      type hints.
- [ ] `stateful` state is `Default`-only, so a form editing existing data
      stores `Option`s that fall back to the data. A `stateful` with an
      initial value.
- [ ] The card stack (fold/unfold, kinetic scroll, shadows on the card
      above) is now copied in two apps: worth a dfui widget with a
      card-painting callback.
- [ ] Color helpers: `mix`/`lighten`/`darken` (fields are public, so apps
      write them inline).

Theming:

- [ ] `dialog`, `bottom_sheet` and `dropdown` are white with dark text on a
      dark app: content inside them must use dark ink and its own key
      colors. Part of Theming.

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
- [ ] Morphs across topology changes (O → C): an unpaired hole shrinks to
      its centre but can leave the outer contour on the way, showing as fill.
- [ ] Layout animations (size/position): need a rebuild or relayout per
      frame while running. List inserts, removals and moves animate
      without one (FLIP offsets).
- [x] Timed wakeups (`PaintCx::request_frame_at`): the caret blink sleeps
      between toggles instead of painting every frame.
- [ ] Theming: shared colors/sizes instead of per-widget constants.
- [x] Images: app-decoded `Bitmap`s, a texture and draw each; demo
      gallery downloads from placehold.co.
- [ ] Image follow-ups: mipmaps, fit modes (contain/cover), a decode
      thread pool.
- [x] Async: `future_builder`/`stream_builder` polled on the UI thread,
      woken through the event loop, cancelled with their node.
- [ ] Async follow-ups: task modifier on `stateful` (results into its own
      state), async event handlers (`cx.spawn`), sync provider reads in
      `stateful` builds, Suspense-style boundaries.
- [ ] Multi-touch: per-finger dispatch (runtime is single-pointer today).
- [x] Kinetic scrolling: a released drag flings with exponential friction,
      stopping at the edges; re-placed per frame via `request_placement`.
- [ ] Horizontal scroll; overscroll bounce/glow; a press that catches a
      fling still reaches the widget under it.
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

- [x] Cache text layouts per (text, size, width), evicted after an unused
      frame. Cut frame CPU ~82%.
- [ ] Cheaper layout misses (changing text: typing, counters): cache shaped
      words, since each word is shaped on its own.
- [x] Run the benches on the phone (`scripts/android-bench.sh`): partial
      frame ~44× cheaper than full on Mali. Still unmeasured in the real
      app: canvas copy and `LoadOp::Load` bandwidth.
- [x] Cheaper shadows: drawn as a ring around the opaque shape on top of
      them (`gpu/cards_10` 156 → 133 µs desktop; no measurable change on
      the Pixel 10a in A/B runs).
- [x] Paint culling: nodes whose box (plus a 48px overdraw margin for
      shadows) is outside their clip aren't painted. 1000 cards: edit frame
      12.1 → 8.5 ms, scroll frame 4.6 → 1.2 ms (desktop).
- [ ] Upload only newly added glyph data (all outlines re-uploaded today).
- [x] Flatten once per rebuild: ids cached, other passes re-walk the tree
      (`docs/tree.md`). Pointer move −57%, scroll frame −21% at 1000 cards.
- [x] Pointer events visit only the nodes under the pointer, the ones it
      just left or pressed, and layers; the walk skips other subtrees.
      Pointer move at 1000 cards 282 → 4.3 µs (a linear scan of the boxes
      is what's left).
- [x] Profile 120 Hz: Pixel 10a at 120 Hz with every frame forced to a full
      redraw holds 120 fps; Bench screen ~3.2 ms GPU / ~1.8 ms CPU, main
      screen ~2.6 ms GPU / ~2.3 ms CPU, against an 8.3 ms budget.

## Tooling

- [x] Golden-image tests (`tests/golden.rs`): widgets rendered offscreen
      on a virtual clock (`Bench::advance`/`settle`), animations as frame
      strips, compared against PNGs from the reference machine.
      `DFUI_BLESS=1 cargo test --test golden` rewrites them.
- [ ] Fold the demo's `#[ignore]` snapshot tests into the goldens.
- [ ] Release signing config for real distribution (debug keystore via
      `CARGO_APK_RELEASE_KEYSTORE` today).
- [ ] cargo-apk is deprecated and capped at NDK platform 35; evaluate
      xbuild or cargo-ndk + Gradle.
