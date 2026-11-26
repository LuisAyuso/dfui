# Number input

`number_input` is a text field for a number in a `range`. What you type
stays as typed, so a year can pass through "2" and "20" on its way to
"2026"; while the text isn't a number in range it shows red and the value
doesn't change. `on_change` gets every valid value, and Enter clamps the
text into range.
