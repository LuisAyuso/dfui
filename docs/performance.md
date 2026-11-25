# Performance

Measured at commit `8f187ba` with the commands in
[development.md](development.md), release builds. Desktop is a Ryzen 5 3600
with an RTX 4060 Ti; the phone has a Mali-G715 GPU. Phone numbers drift
between sessions with GPU clock scaling, so compare builds back to back.

CPU per frame, without the GPU's work (400×800 logical px):

| Bench | Desktop, 100 cards | Phone, 100 cards | Desktop, 1000 cards | Phone, 1000 cards |
|---|---|---|---|---|
| Full frame (`frame/cards`) | 503 µs | 582 µs | 8.77 ms | 11.6 ms |
| One label changes (`frame/edit`) | 562 µs | 643 µs | 9.78 ms | 12.6 ms |
| Scroll (`frame/scroll`) | 131 µs | 133 µs | 1.29 ms | 1.71 ms |
| Pointer move (`frame/pointer_move`) | 0.64 µs | 0.87 µs | 4.8 µs | 6.4 µs |

GPU per frame, render pass only (1080×2400, scale 2.625):

| Bench | RTX 4060 Ti | Mali-G715 |
|---|---|---|
| 10 cards (`gpu/cards_10`) | 133 µs | 3.01 ms |
| Screen of text (`gpu/text_wall`) | 113 µs | 3.65 ms |
| Same, redrawing the FPS meter area (`gpu/text_wall_partial`) | 10.9 µs | 65.8 µs |

Since then every frame reaches the screen through a present pass from a
half-float canvas (for HDR), which the GPU benches now include: on the
Mali-G715 it costs ~190 µs against ~155 µs for the canvas copy it replaced,
so a partial frame measures ~237 µs. See `TODO.md` for the measurements.

Boot:

| | Desktop | Phone |
|---|---|---|
| Runtime and first layout, no window or GPU (`startup/cards_10`) | 644 µs | 578 µs |
| Cold start of the demo (median of 10, 103–121 ms) | not measured | 112 ms |

Cold start is `TotalTime` from `adb shell am start -W`, so it ends when Android
reports the activity displayed, which may be before dfui presents its first frame.
