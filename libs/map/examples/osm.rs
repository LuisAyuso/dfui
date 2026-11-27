//! An OpenStreetMap area as a dfui map: `cargo run -p dfui-map --release
//! --example osm -- [lat lon km] [--bench]`. The download from the Overpass
//! API is cached in the temp directory. The wasm and Android builds embed the
//! JSON at `DFUI_MAP_OSM` instead (see `libs/map/README.md`).

use std::rc::Rc;

use dfui::path::Point;
use dfui::widgets::{Styled, column, text};
use dfui::{App, Color, Element, Font, Text};
use dfui_map::{Layer, MapBuilder, MapData, map};
use serde_json::Value;

pub struct OsmMap {
    data: Rc<MapData>,
    start: Option<(Point, f32)>,
}

impl App for OsmMap {
    fn view(&self) -> Element {
        let m = map(&self.data);
        let m = match self.start {
            Some((c, s)) => m.start(c, s),
            None => m,
        };
        // The data's licence (ODbL) asks for this credit.
        let credit = text("© OpenStreetMap contributors").size(12.0).padding(4.0);
        column([m.fill().into(), credit.into()]).fill().into()
    }
    fn background(&self) -> Color {
        Color::hex(0xf2efe9)
    }
}

struct Style {
    green: Layer,
    water: Layer,
    buildings: Layer,
    rail: Layer,
    paths: Layer,
    minor: Layer,
    major: Layer,
    names: Layer,
}

fn ring(b: &MapBuilder, geometry: &Value) -> Vec<Point> {
    let pts = geometry.as_array().map(Vec::as_slice).unwrap_or_default();
    pts.iter()
        .filter_map(|p| Some(b.point(p["lat"].as_f64()?, p["lon"].as_f64()?)))
        .collect()
}

/// A street name to place: its line, name and road width.
type Name = (Vec<Point>, String, f32);

fn add(b: &mut MapBuilder, s: &Style, names: &mut Vec<Name>, e: &Value) {
    let tag = |k: &str| e["tags"][k].as_str().unwrap_or("");
    let area_layer = match () {
        _ if !tag("building").is_empty() => Some(s.buildings),
        _ if matches!(tag("natural"), "water") || tag("waterway") == "riverbank" => Some(s.water),
        _ if matches!(tag("natural"), "wood") || !tag("leisure").is_empty() => Some(s.green),
        _ if !tag("landuse").is_empty() => Some(s.green),
        _ => None,
    };
    if e["type"] == "relation" {
        let Some(layer) = area_layer else { return };
        let (mut outers, mut holes) = (Vec::new(), Vec::new());
        // ponytail: only member ways that close on themselves; stitch open ones if big lakes go missing.
        for m in e["members"].as_array().into_iter().flatten() {
            let r = ring(b, &m["geometry"]);
            if r.len() > 3 && r.first() == r.last() {
                match m["role"].as_str() {
                    Some("inner") => holes.push(r),
                    _ => outers.push(r),
                }
            }
        }
        b.area(layer, &outers, &holes);
        return;
    }
    let pts = ring(b, &e["geometry"]);
    let road = match tag("highway") {
        "motorway" | "trunk" | "primary" | "secondary" => Some((s.major, 16.0)),
        "tertiary" => Some((s.major, 12.0)),
        "residential" | "unclassified" | "living_street" => Some((s.minor, 9.0)),
        "service" | "pedestrian" => Some((s.minor, 5.0)),
        "" => None,
        _ => Some((s.paths, 2.5)),
    };
    let line = road.or(match tag("waterway") {
        "river" => Some((s.water, 20.0)),
        "canal" => Some((s.water, 10.0)),
        "stream" | "ditch" | "drain" => Some((s.water, 3.0)),
        _ if tag("railway") == "rail" => Some((s.rail, 2.0)),
        _ => None,
    });
    match (line, area_layer) {
        (Some((layer, w)), _) => {
            b.line(layer, &pts, w);
            if let Some(name) = e["tags"]["name"].as_str().filter(|_| road.is_some()) {
                names.push((pts, name.to_owned(), w));
            }
        }
        (None, Some(layer)) if pts.first() == pts.last() => b.area(layer, &[pts], &[]),
        _ => {}
    }
}

/// Central Amsterdam: latitude, longitude and the side of the area in km.
const AMSTERDAM: (f64, f64, f64) = (52.3731, 4.8926, 2.0);

/// The map of [`AMSTERDAM`].
pub fn app() -> OsmMap {
    let (lat, lon, km) = AMSTERDAM;
    OsmMap {
        data: prepare(lat, lon, km),
        start: None,
    }
}

fn main() {
    let args: Vec<f64> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let (lat, lon, km) = match args[..] {
        [lat, lon, km] => (lat, lon, km),
        [lat, lon] => (lat, lon, 2.0),
        _ => AMSTERDAM,
    };
    let data = prepare(lat, lon, km);
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    if std::env::args().any(|a| a == "--bench") {
        return bench(&data);
    }
    dfui::run(OsmMap { data, start: None });
}

fn prepare(lat: f64, lon: f64, km: f64) -> Rc<MapData> {
    let json = load(lat, lon, km);
    let osm: Value = serde_json::from_str(&json).expect("Overpass JSON");
    let t = dfui::Instant::now();
    let mut b = MapBuilder::new(lat, lon);
    let style = Style {
        green: b.layer(Color::hex(0xc8e6b0), 0.0),
        water: b.layer(Color::hex(0xaad3df), 0.0),
        buildings: b.layer(Color::hex(0xd9d0c9), 0.4),
        rail: b.layer(Color::hex(0x9a9a9a), 0.0),
        // Footways, cycleways, steps: under a px wide below this.
        paths: b.layer(Color::WHITE, 0.4),
        minor: b.layer(Color::WHITE, 0.0),
        major: b.layer(Color::hex(0xf8d49c), 0.0),
        names: b.layer(Color::hex(0x5c5852), 0.0),
    };
    let mut names = Vec::new();
    for e in osm["elements"].as_array().into_iter().flatten() {
        add(&mut b, &style, &mut names, e);
    }
    // Earlier labels win the spacing: names of wide roads first.
    names.sort_by(|a, b| b.2.total_cmp(&a.2));
    let text = Text::default();
    for (pts, name, w) in &names {
        b.label(style.names, pts, &text, Font::DEFAULT, name, w * 0.75);
    }
    let data = b.build();
    eprintln!("{} shapes in {:?}", data.tiles(), t.elapsed());
    Rc::new(data)
}

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
fn load(_: f64, _: f64, _: f64) -> String {
    include_str!(env!("DFUI_MAP_OSM")).into()
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
use native::{bench, download as load};

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
mod native {
    use super::*;

    pub fn download(lat: f64, lon: f64, km: f64) -> String {
        let cache = std::env::temp_dir().join(format!("dfui-osm-{lat}-{lon}-{km}.json"));
        if let Ok(s) = std::fs::read_to_string(&cache) {
            return s;
        }
        let (dlat, dlon) = (km / 111.0 / 2.0, km / 111.0 / lat.to_radians().cos() / 2.0);
        let bbox = format!(
            "{},{},{},{}",
            lat - dlat,
            lon - dlon,
            lat + dlat,
            lon + dlon
        );
        let query = format!(
            "[out:json][timeout:90];(
              way[building]({bbox}); way[highway]({bbox}); way[railway=rail]({bbox});
              way[waterway]({bbox}); way[natural~\"^(water|wood)$\"]({bbox});
              way[leisure=park]({bbox}); way[landuse~\"^(grass|forest|meadow)$\"]({bbox});
              relation[natural=water]({bbox}); relation[building]({bbox});
            );out geom;"
        );
        let body = fetch(&query).expect("download");
        let _ = std::fs::write(&cache, &body);
        body
    }

    /// Public Overpass instances; they turn queries away when busy.
    const SERVERS: [&str; 3] = [
        "https://overpass-api.de/api/interpreter",
        "https://overpass.private.coffee/api/interpreter",
        "https://maps.mail.ru/osm/tools/overpass/api/interpreter",
    ];

    /// Tries each server, backing off between rounds as Overpass asks.
    fn fetch(query: &str) -> Result<String, ureq::Error> {
        let mut wait = std::time::Duration::from_secs(2);
        for round in 0..5 {
            for server in SERVERS {
                eprintln!("downloading from {server}…");
                let res = ureq::post(server)
                    .header("User-Agent", "dfui-map-example")
                    .send_form([("data", query)])
                    .and_then(|mut r| r.body_mut().with_config().limit(200 << 20).read_to_string());
                match res {
                    Ok(body) => return Ok(body),
                    Err(e) if round == 4 && server == SERVERS[2] => return Err(e),
                    Err(e) => eprintln!("  {e}"),
                }
            }
            std::thread::sleep(wait);
            wait *= 2;
        }
        unreachable!()
    }

    /// Per-frame CPU paint and GPU time at a few zoom levels, on a phone-sized
    /// target like dfui's own GPU benches.
    pub fn bench(data: &Rc<MapData>) {
        const SIZE: (u32, u32) = (1080, 2400);
        const SCALE: f32 = 2.625;
        let logical = (SIZE.0 as f32 / SCALE, SIZE.1 as f32 / SCALE);
        println!("{} shapes, {} segments", data.tiles(), data.segments());
        println!("| view | CPU paint | GPU frame |\n|---|---|---|");
        let views = [
            None,
            Some(0.3),
            Some(0.5),
            Some(2.0),
            Some(10.0),
            Some(50.0),
        ];
        for scale in views {
            let app = || OsmMap {
                data: data.clone(),
                start: scale.map(|s| ([0.0, 0.0], s)),
            };
            let mut cpu = dfui::Bench::new(app(), logical);
            cpu.settle();
            let t = std::time::Instant::now();
            for _ in 0..200 {
                cpu.paint();
            }
            let paint = t.elapsed() / 200;
            let mut gpu = cpu.gpu(SCALE);
            let mut times: Vec<_> = (0..60).map(|_| gpu.draw(None)).collect();
            times.sort();
            let name = scale.map_or("whole map".into(), |s| format!("{s} px/m"));
            println!("| {name} | {paint:.1?} | {:.2?} |", times[30]);
        }
    }
}
