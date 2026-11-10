# Stream builder

`stream_builder(deps, start, build)` polls the stream that `start` returns
and collects its items into the snapshot's `data`, which can be any
`Default + Extend<Item>` type such as `Vec` or `String`. `done` turns true
when the stream ends. Items that arrive together cost one rebuild.

The stream here is made with `futures_util::stream::unfold` and
`dfui::sleep`, so it runs in the browser without threads.
