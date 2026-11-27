//! Weather for Spain (AEMET) and Austria (GeoSphere Austria) on a map of
//! their regions, with icons from Lucide.

mod forecast;

use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use dfui::path::Point;
use dfui::taffy::AlignItems;
use dfui::widgets::{Styled, column, icon, row, segmented, spacer, stateful, text, watch};
use dfui::{App, Color, Element, Font, Rect, Shape};
use dfui_map::{MapBuilder, MapData, map};
use serde_json::Value;

use forecast::{Day, STORE, Sky};

pub struct City {
    pub name: &'static str,
    pub lat: f64,
    pub lon: f64,
    /// INE municipality code, AEMET's id; empty outside Spain.
    pub aemet: &'static str,
}

const fn city(name: &'static str, lat: f64, lon: f64, aemet: &'static str) -> City {
    City {
        name,
        lat,
        lon,
        aemet,
    }
}

struct Country {
    name: &'static str,
    origin: (f64, f64),
    /// NUTS 2 regions from Eurostat GISCO, 1:3M.
    regions: &'static str,
    cities: &'static [City],
    credit: &'static str,
}

const COUNTRIES: [Country; 2] = [
    Country {
        name: "España",
        origin: (40.0, -3.7),
        regions: include_str!("../assets/es.geojson"),
        cities: &[
            city("Madrid", 40.4168, -3.7038, "28079"),
            city("Barcelona", 41.3874, 2.1686, "08019"),
            city("Valencia", 39.4699, -0.3763, "46250"),
            city("Sevilla", 37.3891, -5.9845, "41091"),
            city("Zaragoza", 41.6488, -0.8891, "50297"),
            city("Málaga", 36.7213, -4.4214, "29067"),
            city("Murcia", 37.9922, -1.1307, "30030"),
            city("Palma", 39.5696, 2.6502, "07040"),
            city("Las Palmas", 28.1235, -15.4363, "35016"),
            city("Santa Cruz de Tenerife", 28.4636, -16.2518, "38038"),
            city("Bilbao", 43.2630, -2.9350, "48020"),
            city("Valladolid", 41.6523, -4.7245, "47186"),
            city("A Coruña", 43.3623, -8.4115, "15030"),
            city("Oviedo", 43.3614, -5.8494, "33044"),
            city("Santander", 43.4623, -3.8100, "39075"),
            city("Pamplona", 42.8125, -1.6458, "31201"),
            city("Granada", 37.1773, -3.5986, "18087"),
            city("Alicante", 38.3452, -0.4810, "03014"),
            city("Badajoz", 38.8794, -6.9707, "06015"),
            city("Salamanca", 40.9701, -5.6635, "37274"),
        ],
        credit: "Datos: © AEMET · Límites: © EuroGeographics",
    },
    Country {
        name: "Österreich",
        origin: (47.6, 13.3),
        regions: include_str!("../assets/at.geojson"),
        cities: &[
            city("Wien", 48.2082, 16.3738, ""),
            city("Graz", 47.0707, 15.4395, ""),
            city("Linz", 48.3069, 14.2858, ""),
            city("Salzburg", 47.8095, 13.0550, ""),
            city("Innsbruck", 47.2692, 11.4041, ""),
            city("Klagenfurt", 46.6247, 14.3053, ""),
            city("St. Pölten", 48.2047, 15.6256, ""),
            city("Bregenz", 47.5031, 9.7471, ""),
            city("Eisenstadt", 47.8457, 16.5233, ""),
            city("Lienz", 46.8297, 12.7697, ""),
        ],
        credit: "Daten: GeoSphere Austria · Grenzen: © EuroGeographics",
    },
];

const SEA: Color = Color::hex(0xdce8ef);
const LAND: Color = Color::hex(0xf4f1ea);
const BORDER: Color = Color::hex(0xb8b0a2);
const INK: Color = Color::hex(0x2b2d33);
const MUTED: Color = Color::hex(0x6b7080);
const ACCENT: Color = Color::hex(0x1a73e8);
/// Region borders, km wide: dfui-map strokes in map units.
const BORDER_KM: f32 = 1.2;
/// A tap selects the closest city within this many km.
const REACH_KM: f32 = 60.0;

/// The Canary Islands moved next to the peninsula, as Spanish maps inset them.
fn inset(lat: f64, lon: f64) -> (f64, f64) {
    if lat < 30.0 && lon < -12.0 {
        (lat + 6.0, lon + 6.0)
    } else {
        (lat, lon)
    }
}

/// Map units are km, not dfui-map's metres: its tiles are sized for cities,
/// and a country in metres would split into far too many of them.
fn km(b: &MapBuilder, lat: f64, lon: f64) -> Point {
    let (lat, lon) = inset(lat, lon);
    let [x, y] = b.point(lat, lon);
    [x / 1000.0, y / 1000.0]
}

struct Place {
    country: &'static Country,
    data: Rc<MapData>,
    cities: Vec<Point>,
}

fn place(country: &'static Country) -> Place {
    let mut b = MapBuilder::new(country.origin.0, country.origin.1);
    let land = b.layer(LAND, 0.0);
    let border = b.layer(BORDER, 0.0);
    let geo: Value = serde_json::from_str(country.regions).expect("regions geojson");
    for f in geo["features"].as_array().into_iter().flatten() {
        let g = &f["geometry"];
        let polygons: Vec<&Value> = match g["type"].as_str() {
            Some("Polygon") => vec![&g["coordinates"]],
            _ => g["coordinates"].as_array().into_iter().flatten().collect(),
        };
        let (mut outers, mut holes) = (Vec::new(), Vec::new());
        for poly in polygons {
            for (k, ring) in poly.as_array().into_iter().flatten().enumerate() {
                let pts: Vec<Point> = (ring.as_array().into_iter().flatten())
                    .filter_map(|xy| Some(km(&b, xy[1].as_f64()?, xy[0].as_f64()?)))
                    .collect();
                b.line(border, &pts, BORDER_KM);
                if k == 0 {
                    outers.push(pts)
                } else {
                    holes.push(pts)
                }
            }
        }
        b.area(land, &outers, &holes);
    }
    let cities = country
        .cities
        .iter()
        .map(|c| km(&b, c.lat, c.lon))
        .collect();
    Place {
        country,
        data: Rc::new(b.build()),
        cities,
    }
}

/// Lucide icons, converted with svg2path; indexed by [`Sky`].
struct Icons {
    sky: [Shape; 7],
    droplets: Shape,
}

fn icons() -> Icons {
    macro_rules! load {
        ($name:literal) => {
            Shape::from_bytes(include_bytes!(concat!("../assets/icons/", $name, ".path")))
                .expect($name)
        };
    }
    Icons {
        sky: [
            load!("sun"),
            load!("cloud-sun"),
            load!("cloud"),
            load!("cloud-fog"),
            load!("cloud-rain"),
            load!("cloud-snow"),
            load!("cloud-lightning"),
        ],
        droplets: load!("droplets"),
    }
}

#[derive(Default)]
struct Ui {
    country: usize,
    city: Option<usize>,
}

struct Meteo {
    places: Rc<[Place; 2]>,
    icons: Rc<Icons>,
}

impl App for Meteo {
    fn view(&self) -> Element {
        let (places, icons) = (self.places.clone(), self.icons.clone());
        watch(&STORE.notifier, move || {
            let (places, icons) = (places.clone(), icons.clone());
            stateful(move |ui: &Ui, h| screen(&places[ui.country], &icons, ui, h)).fill()
        })
        .fill()
        .into()
    }

    fn background(&self) -> Color {
        SEA
    }
}

fn screen(place: &Place, icons: &Rc<Icons>, ui: &Ui, h: dfui::Handle<Ui>) -> Element {
    let country = place.country;
    let forecasts: Vec<Option<forecast::Forecast>> = {
        let data = STORE.data.lock().unwrap();
        country
            .cities
            .iter()
            .map(|c| data.get(c.name).cloned())
            .collect()
    };
    // Today's sky and high per city: "…" while loading, "–" without data.
    let today: Vec<(Option<Sky>, String)> = (forecasts.iter())
        .map(
            |f| match f.as_ref().map(|f| f.as_ref().ok().and_then(|d| d.first())) {
                None => (None, "…".into()),
                Some(None) => (None, "–".into()),
                Some(Some(d)) => (Some(d.sky), format!("{:.0}°", d.max)),
            },
        )
        .collect();

    let (points, chips, selected) = (place.cities.clone(), icons.clone(), ui.city);
    let taps = place.cities.clone();
    let m = map(&place.data)
        .on_tap(move |cx, p| {
            let d = |c: &Point| (c[0] - p[0]).hypot(c[1] - p[1]);
            let near = (0..taps.len()).min_by(|&a, &b| d(&taps[a]).total_cmp(&d(&taps[b])));
            if let Some(i) = near.filter(|&i| d(&taps[i]) < REACH_KM) {
                cx.update(h, move |u: &mut Ui| u.city = Some(i));
            }
        })
        .overlay(move |cx, to_px| {
            for (i, (&p, (sky, label))) in points.iter().zip(&today).enumerate() {
                let [x, y] = to_px(p);
                let w = 30.0 + cx.measure(label, Font::BOLD, 13.0, None).width;
                let r = Rect::new(x - w / 2.0, y - 13.0, w, 26.0);
                let border = if selected == Some(i) { 2.0 } else { 0.5 };
                cx.fill_bordered(
                    r,
                    13.0,
                    Color::WHITE,
                    border,
                    if border > 1.0 { ACCENT } else { BORDER },
                );
                if let Some(sky) = sky {
                    cx.shape(
                        &chips.sky[*sky as usize],
                        Rect::new(r.x + 6.0, r.y + 4.0, 18.0, 18.0),
                        INK,
                    );
                }
                cx.text(label, Font::BOLD, 13.0, INK, None, (r.x + 26.0, r.y + 5.0));
            }
        });

    let tabs = segmented(COUNTRIES.iter().map(|c| c.name), ui.country)
        .on_change(move |cx, i| cx.update(h, move |u: &mut Ui| (u.country, u.city) = (i, None)));
    let mut panel: Vec<Element> = vec![tabs.into()];
    match ui.city {
        None => panel.push(text("Tap a city on the map").color(MUTED).into()),
        Some(i) => {
            panel.push(
                text(country.cities[i].name)
                    .font(Font::BOLD)
                    .size(20.0)
                    .into(),
            );
            match &forecasts[i] {
                None => panel.push(text("Loading…").color(MUTED).into()),
                Some(Err(e)) => panel.push(text(e).size(13.0).color(Color::hex(0xc62828)).into()),
                Some(Ok(days)) => panel.extend(days.iter().map(|d| day_row(d, icons))),
            }
        }
    }
    panel.push(spacer().grow(1.0).into());
    panel.push(text(country.credit).size(11.0).color(MUTED).into());

    row([
        // Keyed so each country keeps its own pan and zoom.
        Element::from(m.width(0.0).grow(1.0)).key(ui.country as u64),
        column(panel)
            .gap(12.0)
            .padding(16.0)
            .width(320.0)
            .shrink(0.0)
            .bg(Color::WHITE)
            .into(),
    ])
    .fill()
    .into()
}

fn day_row(d: &Day, icons: &Icons) -> Element {
    let today = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        / 86_400) as i64;
    let name = match d.day - today {
        0 => "Today",
        1 => "Tomorrow",
        _ => forecast::weekday(d.day),
    };
    row([
        text(name).width(80.0).into(),
        icon(&icons.sky[d.sky as usize])
            .size(22.0)
            .color(INK)
            .into(),
        text(format!("{:.0}° / {:.0}°", d.max, d.min))
            .grow(1.0)
            .into(),
        icon(&icons.droplets).size(14.0).color(ACCENT).into(),
        text(&d.rain).size(13.0).color(MUTED).into(),
    ])
    .gap(10.0)
    .align(AlignItems::CENTER)
    .into()
}

fn main() {
    forecast::start(COUNTRIES[0].cities, COUNTRIES[1].cities);
    let places = Rc::new([place(&COUNTRIES[0]), place(&COUNTRIES[1])]);
    dfui::run(Meteo {
        places,
        icons: Rc::new(icons()),
    });
}
