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
- [ ] Partial redraws: avoid full rebuild/relayout/repaint on every change.
      Open questions: rebuild only dirty `stateful` subtrees; cache taffy
      nodes for incremental layout; retained scene with damage rects vs
      redrawing the whole frame; interaction with `request_frame`
      animations and power use on mobile. GPU side done (scissored canvas,
      per-node scene diff, idle frames skipped; `docs/rendering.md`);
      measure on the phone next. CPU side (rebuild/layout/paint) still full every change.

## Framework

- [ ] Animations: build on `PaintCx::request_frame`; time source + easing.
- [ ] Theming: shared colors/sizes instead of per-widget constants.
- [ ] Images.
- [ ] Async commands/subscriptions (Elm `Cmd`/`Sub`).
- [ ] Multi-touch: per-finger dispatch (runtime is single-pointer today).
- [ ] Kinetic scrolling and horizontal scroll.
- [ ] Widget library: text field, checkbox, switch, slider, list, tabs.

## Text

- [ ] Shaping and bidi (rustybuzz); glyph cache keyed by glyph id, not `char`.
- [ ] Multiple fonts, bold/italic, fallback.
- [ ] Line breaking beyond spaces (long words/URLs).
- [ ] Adaptive cubic→quadratic split for CFF fonts (fixed 4-way split today).
- [ ] Test with variable fonts / overlapping contours.
- [ ] True signed distance (cubic solve) if text outlines/glow/blurred shadows
      are needed; current shader computes coverage only.

## Performance

CPU baseline (`cargo bench`, desktop): one frame of rebuild + layout + paint
+ diff costs ~1.5 ms for 10 cards and scales linearly (~150 µs/card, 167 ms
for 1000); laying out one 300-char paragraph costs ~66 µs.

- [ ] Cache text layout/measurement: `Text::layout` re-queries advances and
      kerning per char on every measure (taffy calls it several times per
      leaf) and again in paint. Likely the dominant CPU cost.
- [ ] Skip layout/paint of off-screen scroll content.
- [ ] Upload only newly added glyph data (all outlines re-uploaded today).
- [ ] Incremental layout: cache taffy nodes instead of full relayout per frame.
- [ ] Event hit-testing index (every event visits every node today).
- [ ] Profile 120 Hz devices; outline text costs ~1.6 ms more than the old
      MSDF path on a typical screen.

## Tooling

- [ ] Golden-image tests on `dfui::snapshot` (currently eyeballed, `#[ignore]`).
- [ ] Release signing config for real distribution (debug keystore via
      `CARGO_APK_RELEASE_KEYSTORE` today).
- [ ] cargo-apk is deprecated and capped at NDK platform 35; evaluate
      xbuild or cargo-ndk + Gradle.
