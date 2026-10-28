# dfui roadmap

Baseline (Pixel 10a, Mali-G715, release build): bench screen of 64dp text at
60 fps, ~5.1 ms GPU / ~2.2 ms CPU per frame; normal demo screen ~4.5 ms GPU.

## Next

- [ ] Text input + keyboard focus: focus model in widget state, Android soft
      keyboard/IME via android-activity, caret and selection painting.
- [ ] Faster outline text: store curves inline in each band list (drop the
      index indirection, one fewer dependent load per curve). Re-measure with
      the Bench screen.
- [ ] Status bar icon color: light/dark appearance so icons stay visible on
      light backgrounds.

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
