# dfui roadmap

Baseline (Pixel 10a, Mali-G715, release build): bench screen of 64dp text at
60 fps, ~5.1 ms GPU / ~2.2 ms CPU per frame; normal demo screen ~4.5 ms GPU.

## Next

- [x] Text input + keyboard focus: single-line `text_input`, tap to focus,
      Android soft keyboard, caret.
- [ ] Text input follow-ups: selection, clipboard, caret blink, desktop IME
      composition, multi-line, keyboard-aware insets on Android.
- [ ] Faster outline text: store curves inline in each band list (drop the
      index indirection, one fewer dependent load per curve). Re-measure with
      the Bench screen.
- [ ] Status bar icon color: light/dark appearance so icons stay visible on
      light backgrounds.

## Design studies (need discussion before code)

- [ ] Global state management: how app-wide state reaches deep widgets
      without threading it through every `view` call or `Msg`. Open
      questions: Elm `App` vs context/provider-style injection vs shared
      stores; how it coexists with `stateful` local state; change
      notification and which subtrees rebuild.
- [x] Partial repaint (GPU): scissored retained canvas, per-node damage
      from a scene diff, idle frames skipped (`docs/rendering.md`). Still to
      measure on the phone against the Bench baseline.
- [ ] Partial rebuild and relayout (CPU): every rebuild still runs the whole
      `App::view`, a full taffy layout and a full paint (redraws without a
      rebuild reuse taffy's result: 1000-card scroll frame 31 → 6 ms). Open questions:
      rebuild only dirty `stateful` subtrees; cache taffy nodes for
      incremental layout; skip off-screen scroll content; how this interacts
      with global state change notification.

## Framework

- [ ] Animations: build on `PaintCx::request_frame`; time source + easing.
- [ ] Theming: shared colors/sizes instead of per-widget constants.
- [ ] Images.
- [ ] Async commands/subscriptions (Elm `Cmd`/`Sub`).
- [ ] Multi-touch: per-finger dispatch (runtime is single-pointer today).
- [ ] Kinetic scrolling and horizontal scroll.
- [ ] Widget library: checkbox, switch, slider, list, tabs.

## Text

- [x] Multiple fonts: app-supplied `Font` bytes (bold/italic are just more
      fonts), glyphs keyed by (font, glyph id), fallback to the default font.
- [x] Shaping with harfrust (read-fonts replaces ttf-parser): GPOS kerning,
      ligatures, marks, Arabic joining. Each word is shaped on its own.
- [ ] Bidi (unicode-bidi): mixed-direction lines come out in logical order.
- [ ] Caret moves by grapheme in `text_input` (moves by char today, so it
      can land inside a ligature or between a base and its mark).
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
- [ ] Cheaper shadows: cards outweigh a page of glyphs on the GPU bench.
- [ ] Upload only newly added glyph data (all outlines re-uploaded today).
- [ ] Event hit-testing index (every event visits every node today).
- [ ] Profile 120 Hz devices; outline text costs ~1.6 ms more than the old
      MSDF path on a typical screen.

## Tooling

- [ ] Golden-image tests on `dfui::snapshot` (currently eyeballed, `#[ignore]`).
- [ ] Release signing config for real distribution (debug keystore via
      `CARGO_APK_RELEASE_KEYSTORE` today).
- [ ] cargo-apk is deprecated and capped at NDK platform 35; evaluate
      xbuild or cargo-ndk + Gradle.
