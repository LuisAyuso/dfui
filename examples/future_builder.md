# Future builder

`future_builder(deps, start, build)` runs the future that `start` returns
and rebuilds when it resolves; until then the snapshot's `data` is `None`.
Changing `deps` drops the running future and starts a new one, and
removing the widget cancels it.

`dfui::sleep` stands in for a network request; any std `Future` works,
from any runtime.
