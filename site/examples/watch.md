# Watch

`watch(&notifier, || view)` rebuilds its subtree each time the `Notifier`
is notified. Any thread may call `notify`, so it suits data that changes
outside the UI, such as a download's progress or a store. Here a button
stands in for the outside world.
