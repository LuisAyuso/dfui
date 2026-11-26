# dfui-bevy (experiment)

A [dfui](https://github.com/LuisAyuso/dfui) app as the HUD of a
[Bevy](https://bevyengine.org) 0.20 game. dfui draws with Bevy's own wgpu
device into an `Image` shown as a full-window UI node over everything else,
and reads the window's raw winit events, so text input, focus, scrolling and
gestures behave as in a dfui window.

```sh
cargo run -p dfui-bevy --release --example orbits
```

The example puts a dfui panel over planets orbiting a sun: speed, pause, add
and clear go to the game, and clicks that miss the panel drop a planet.

## Use

```rust,ignore
App::new()
    .add_plugins(DefaultPlugins)
    .add_plugins(DfuiPlugin::new(move || Hud { tx, ..default() }))
```

- The app's `background` should be `Color::TRANSPARENT`; the game shows
  wherever the UI paints nothing.
- Game state goes in through `NonSendMut<Dfui<A>>` and `Embedded::app`,
  which rebuilds the view. Handlers run on the main thread and send
  commands back, e.g. over a channel.
- `UiTookInput` is true in a frame where the pointer was over something the
  UI drew, a widget captured it, or a text field has focus: game systems
  skip their input then.

## Limits

- One window: every window's events go to the overlay.
- The overlay redraws every frame, and is drawn into its image in the main
  world, a frame ahead of the render world after a resize.
- The full-window UI node takes part in Bevy's picking; give it
  `Pickable::IGNORE` if the game picks entities.
