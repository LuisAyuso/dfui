//! The `play` example as an Android app, playing a fragmented MP4 embedded from
//! `DFUI_MEDIA_CLIP`.

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: dfui::AndroidApp) {
    use dfui::Element;
    use dfui_media::{Player, video};

    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
    static CLIP: &[u8] = include_bytes!(env!("DFUI_MEDIA_CLIP"));
    thread_local! {
        static PLAYER: Player = Player::new(CLIP, 0.0);
    }
    fn view() -> Element {
        video(&PLAYER.with(Player::clone)).into()
    }
    dfui::run_android(view, app);
}
