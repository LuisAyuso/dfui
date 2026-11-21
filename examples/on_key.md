# On key

`on_key(f)` gets the keys pressed while no text field is focused: arrows and
Enter from a keyboard, a TV remote or a gamepad's D-pad. Handlers run topmost
first; call `cx.stop()` for a key you used. Text fields take Enter through
`on_submit`.
