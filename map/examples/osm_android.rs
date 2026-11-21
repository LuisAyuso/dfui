//! The `osm` example as an Android app, with the OSM JSON embedded from
//! `DFUI_MAP_OSM` (see `map/README.md`).

#[cfg(target_os = "android")]
#[allow(dead_code)]
#[path = "osm.rs"]
mod osm;

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: dfui::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
    dfui::run_android(osm::app(), app);
}
