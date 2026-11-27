//! Plays a fragmented MP4 file (H.264 + AAC), as the movies-thing server streams them.
//! Space pauses. `cargo run -p dfui-media --example play -- movie.mp4`

use dfui::widgets::{Styled, column, on_key};
use dfui::{Element, Key};
use dfui_media::{Player, video};

thread_local! {
    static PLAYER: Player = {
        let path = std::env::args().nth(1).expect("usage: play <file.mp4>");
        Player::new(std::fs::File::open(path).expect("open the file"), 0.0)
    };
}

fn view() -> Element {
    let player = PLAYER.with(Player::clone);
    let p = player.clone();
    column([
        on_key(move |cx, k| {
            if k == Key::Char(' ') {
                p.set_paused(!p.paused());
                cx.stop();
            }
        })
        .into(),
        video(&player).into(),
    ])
    .fill()
    .into()
}

/// Shows HDR as HDR where the screen takes it.
struct Play;

impl dfui::App for Play {
    fn view(&self) -> Element {
        view()
    }
    fn hdr(&self) -> bool {
        true
    }
}

fn main() {
    env_logger::init();
    dfui::run(Play);
}
