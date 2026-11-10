# Provide

`provide(value, child)` makes `value` available to everything below it.
Builders read the nearest one with `scope.get::<T>()` when they start their
source, so external state (clients, channels, stores) is handed down
without passing it through every function.

Here a provided `Greeter` service answers after a short delay.
