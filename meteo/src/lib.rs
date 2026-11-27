//! Weather for Spain (AEMET) and Austria (GeoSphere Austria) on a map of
//! their regions, with icons from Lucide. Phone-shaped: the map on the top
//! third, the forecast table below.

mod cities;
mod forecast;

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use dfui::path::Point;
use dfui::taffy::AlignItems;
use dfui::widgets::{
    Styled, backdrop, button, column, icon, icon_button, row, scroll, segmented, stateful_with,
    text, wait_bar, watch,
};
use dfui::{App, Color, Element, EventCx, Font, Handle, PaintCx, Rect, Shape};
use dfui_map::{MapBuilder, MapData, map};
use serde_json::Value;

pub use cities::City;
use forecast::{Day, STORE, Sky};

struct Country {
    name: &'static str,
    origin: (f64, f64),
    /// NUTS 2 regions from Eurostat GISCO, 1:3M.
    regions: &'static str,
    cities: &'static [City],
    credit: &'static str,
    words: Words,
}

/// The interface in the country's language.
struct Words {
    capitals: &'static str,
    cities: &'static str,
    loading: &'static str,
    loading_hours: &'static str,
    no_hours: &'static str,
    today: &'static str,
    tomorrow: &'static str,
    /// From Monday.
    weekdays: [&'static str; 7],
}

const COUNTRIES: [Country; 2] = [
    Country {
        name: "España",
        origin: (40.0, -3.7),
        regions: include_str!("../assets/es.geojson"),
        cities: cities::SPAIN,
        credit: "Datos: © AEMET · Límites: © EuroGeographics",
        words: Words {
            capitals: "Capitales autonómicas",
            cities: "Ciudades",
            loading: "Cargando…",
            loading_hours: "Cargando horas…",
            no_hours: "Sin previsión por horas para este día",
            today: "Hoy",
            tomorrow: "Mañana",
            weekdays: ["Lun", "Mar", "Mié", "Jue", "Vie", "Sáb", "Dom"],
        },
    },
    Country {
        name: "Österreich",
        origin: (47.6, 13.3),
        regions: include_str!("../assets/at.geojson"),
        cities: cities::AUSTRIA,
        credit: "Daten: GeoSphere Austria · Grenzen: © EuroGeographics",
        words: Words {
            capitals: "Landeshauptstädte",
            cities: "Städte",
            loading: "Wird geladen…",
            loading_hours: "Stunden werden geladen…",
            no_hours: "Keine Stundenprognose für diesen Tag",
            today: "Heute",
            tomorrow: "Morgen",
            weekdays: ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"],
        },
    },
];

const SEA: Color = Color::hex(0xdce8ef);
const LAND: Color = Color::hex(0xf4f1ea);
const BORDER: Color = Color::hex(0xb8b0a2);
const INK: Color = Color::hex(0x2b2d33);
const MUTED: Color = Color::hex(0x6b7080);
const ACCENT: Color = Color::hex(0x1a73e8);
const ERROR: Color = Color::hex(0xc62828);
/// Behind an open day and its hours.
const TINT: Color = Color::hex(0xeef2f7);
/// Region borders, km wide: dfui-map strokes in map units.
const BORDER_KM: f32 = 1.2;
/// A tap on the map picks the closest city within this many px.
const REACH_PX: f32 = 30.0;
/// Room left around a fitted region, as a share of its size.
const FIT_MARGIN: f32 = 0.12;

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
    /// What the map fits: the country, and each region by name.
    bounds: Rect,
    regions: HashMap<&'static str, Rect>,
}

/// The box around `pts`, grown by [`FIT_MARGIN`].
fn around<'a>(pts: impl Iterator<Item = &'a Point>) -> Rect {
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in pts {
        (lo[0], lo[1]) = (lo[0].min(p[0]), lo[1].min(p[1]));
        (hi[0], hi[1]) = (hi[0].max(p[0]), hi[1].max(p[1]));
    }
    let (w, h) = (hi[0] - lo[0], hi[1] - lo[1]);
    let m = w.max(h) * FIT_MARGIN;
    Rect::new(lo[0] - m, lo[1] - m, w + 2.0 * m, h + 2.0 * m)
}

fn place(country: &'static Country) -> Place {
    let mut b = MapBuilder::new(country.origin.0, country.origin.1);
    let land = b.layer(LAND, 0.0);
    let border = b.layer(BORDER, 0.0);
    let geo: Value = serde_json::from_str(country.regions).expect("regions geojson");
    let mut regions = HashMap::new();
    let mut all = Vec::new();
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
        // The cities name their region, so the lookup borrows their names.
        let name = f["properties"]["name"].as_str().unwrap_or_default();
        if let Some(c) = country.cities.iter().find(|c| c.region == name) {
            regions.insert(c.region, around(outers.iter().flatten()));
        }
        all.extend(outers.iter().flatten().copied());
        b.area(land, &outers, &holes);
    }
    let cities = (country.cities.iter())
        .map(|c| km(&b, c.lat, c.lon))
        .collect();
    Place {
        country,
        data: Rc::new(b.build()),
        cities,
        bounds: around(all.iter()),
        regions,
    }
}

/// Lucide icons, converted with svg2path; `sky` indexed by [`Sky`].
struct Icons {
    sky: [Shape; 7],
    droplets: Shape,
    wind: Shape,
    /// Pointing north, then clockwise by compass point.
    arrows: [Shape; 8],
    back: Shape,
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
        wind: load!("wind"),
        arrows: [
            load!("arrow-up"),
            load!("arrow-up-right"),
            load!("arrow-right"),
            load!("arrow-down-right"),
            load!("arrow-down"),
            load!("arrow-down-left"),
            load!("arrow-left"),
            load!("arrow-up-left"),
        ],
        back: load!("chevron-left"),
    }
}

#[derive(Default)]
struct Ui {
    country: usize,
    /// The region the map is zoomed to, showing all its cities.
    region: Option<&'static str>,
    city: Option<usize>,
    /// The day whose hours are open.
    day: Option<i64>,
}

pub struct Meteo {
    places: Rc<[Place; 2]>,
    icons: Rc<Icons>,
}

impl App for Meteo {
    fn view(&self) -> Element {
        let (places, icons) = (self.places.clone(), self.icons.clone());
        watch(&STORE.notifier, move || {
            let (places, icons) = (places.clone(), icons.clone());
            // Opens on Tirol, zoomed in.
            let start = || Ui {
                country: 1,
                region: Some("Tirol"),
                ..Ui::default()
            };
            stateful_with(start, move |ui: &Ui, h| {
                screen(&places[ui.country], &icons, ui, h)
            })
            .fill()
        })
        .fill()
        .into()
    }

    fn background(&self) -> Color {
        SEA
    }
}

/// Opens city `i`: zooms to its region.
fn open(cx: &mut EventCx, h: Handle<Ui>, cities: &'static [City], i: usize) {
    let c = &cities[i];
    cx.update(h, move |u: &mut Ui| {
        (u.region, u.city) = (Some(c.region), Some(i))
    });
}

/// Up one level: from a city to its region's table, from there to the country.
fn back(u: &mut Ui) {
    if u.city.is_some() {
        (u.city, u.day) = (None, None);
    } else {
        u.region = None;
    }
}

/// A city's sky and high today: "…" while loading, "–" without data.
fn today(f: Option<&forecast::Forecast>) -> (Option<Sky>, String) {
    match f.map(|f| f.as_ref().ok().and_then(|d| d.first())) {
        None => (None, "…".into()),
        Some(None) => (None, "–".into()),
        Some(Some(d)) => (Some(d.sky), format!("{}°", deg(d.max))),
    }
}

fn screen(place: &Place, icons: &Rc<Icons>, ui: &Ui, h: Handle<Ui>) -> Element {
    let country = place.country;
    let cities = country.cities;
    let forecasts: Vec<Option<forecast::Forecast>> = {
        let data = STORE.data.lock().unwrap();
        cities.iter().map(|c| data.get(c.name).cloned()).collect()
    };
    // Capitals always, and every city of the open region.
    let shown: Vec<usize> = (0..cities.len())
        .filter(|&i| cities[i].capital || Some(cities[i].region) == ui.region)
        .collect();
    let fit = (ui.region.and_then(|r| place.regions.get(r)))
        .copied()
        .unwrap_or(place.bounds);

    let px_per_km = Rc::new(Cell::new(1.0));
    let (scale, points, taps) = (px_per_km.clone(), place.cities.clone(), shown.clone());
    let marks: Vec<Mark> = (shown.iter())
        .map(|&i| Mark {
            at: place.cities[i],
            today: today(forecasts[i].as_ref()),
            name: (ui.region == Some(cities[i].region)).then_some(cities[i].name),
            selected: ui.city == Some(i),
        })
        .collect();
    let chips = icons.clone();
    let m = map(&place.data)
        .fit(fit)
        .on_tap(move |cx, p| {
            let d = |i: usize| (points[i][0] - p[0]).hypot(points[i][1] - p[1]);
            let near = taps.iter().copied().min_by(|&a, &b| d(a).total_cmp(&d(b)));
            if let Some(i) = near.filter(|&i| d(i) * scale.get() < REACH_PX) {
                open(cx, h, cities, i);
            }
        })
        .overlay(move |cx, to_px| {
            px_per_km.set(to_px([1.0, 0.0])[0] - to_px([0.0, 0.0])[0]);
            draw_marks(cx, &marks, &chips, to_px);
        });

    let tabs = segmented(COUNTRIES.iter().map(|c| c.name), ui.country).on_change(move |cx, i| {
        cx.update(h, move |u: &mut Ui| {
            *u = Ui {
                country: i,
                ..Ui::default()
            }
        })
    });
    let (title, subtitle) = match (ui.city, ui.region) {
        (Some(i), _) => (cities[i].name, cities[i].region),
        (None, Some(r)) => (r, country.words.cities),
        (None, None) => (country.name, country.words.capitals),
    };
    let mut head: Vec<Element> = Vec::new();
    if ui.region.is_some() {
        let b = icon_button(&icons.back).color(INK);
        head.push(b.on_click(move |cx| cx.update(h, back)).into());
    }
    head.push(
        column([
            text(title).font(Font::BOLD).size(20.0).color(INK).into(),
            text(subtitle).size(13.0).color(MUTED).into(),
        ])
        .into(),
    );

    let list: Vec<Element> = match ui.city {
        Some(i) => days(
            &forecasts[i],
            ui.day,
            icons,
            h,
            cities[i].name,
            &country.words,
        ),
        None => (shown.iter())
            .filter(|&&i| ui.region.is_none_or(|r| cities[i].region == r))
            .map(|&i| {
                let row = city_row(cities[i].name, &forecasts[i], icons);
                let b = button(row).flat().color(Color::WHITE).fill_width();
                b.on_click(move |cx| open(cx, h, cities, i)).into()
            })
            .collect(),
    };

    let mut rows = vec![tabs.into()];
    let aemet = country.cities.iter().any(|c| !c.aemet.is_empty());
    if aemet && STORE.throttled.load(Ordering::Relaxed) {
        rows.push(
            column([
                wait_bar().accent(ACCENT).fill_width().into(),
                text("AEMET limita las consultas; se reintenta en un minuto")
                    .size(12.0)
                    .color(MUTED)
                    .into(),
            ])
            .gap(4.0)
            .fill_width()
            .into(),
        );
    }
    rows.extend([
        row(head).gap(4.0).align(AlignItems::CENTER).into(),
        scroll(column(list).gap(4.0)).grow(1.0).into(),
        text(country.credit).size(11.0).color(MUTED).into(),
    ]);
    let panel = column(rows).gap(10.0).padding(12.0).bg(Color::WHITE);
    // Edge to edge: the map runs under the status bar, the panel under the
    // navigation bar with its content kept clear of it.
    let bottom = dfui::insets()[2];
    let mut panel = panel.height(0.0).grow(2.0);
    panel.style_mut().padding.bottom = dfui::taffy::LengthPercentage::length(12.0 + bottom);
    backdrop(
        column([
            // Keyed so each country keeps its own pan and zoom.
            Element::from(m.height(0.0).grow(1.0)).key(ui.country as u64),
            panel.into(),
        ])
        .fill(),
    )
    .into()
}

/// A city on the map: where, today's weather, and its name once its region
/// is open.
struct Mark {
    at: Point,
    today: (Option<Sky>, String),
    name: Option<&'static str>,
    selected: bool,
}

/// Chips for the marks; one that would overlap a chip already drawn is
/// left as a dot, the selected one going first.
fn draw_marks(cx: &mut PaintCx, marks: &[Mark], icons: &Icons, to_px: &dyn Fn(Point) -> Point) {
    let mut order: Vec<&Mark> = marks.iter().collect();
    order.sort_by_key(|m| !m.selected);
    // Chips where they fit; the crowded-out marks as dots, except under a chip.
    let mut taken: Vec<Rect> = Vec::new();
    let (mut chips, mut dots) = (Vec::new(), Vec::new());
    for m in order {
        let [x, y] = to_px(m.at);
        let w = 30.0 + cx.measure(&m.today.1, Font::BOLD, 13.0, None).width;
        let chip = Rect::new(x - w / 2.0, y - 13.0, w, 26.0);
        let name_h = if m.name.is_some() { 14.0 } else { 0.0 };
        let space = Rect::new(chip.x, chip.y, chip.w, chip.h + name_h);
        if taken.iter().any(|t| overlaps(t, &space)) {
            dots.push(Rect::new(x - 3.0, y - 3.0, 6.0, 6.0));
        } else {
            taken.push(space);
            chips.push((m, x, chip));
        }
    }
    for d in dots {
        if !taken.iter().any(|t| overlaps(t, &d)) {
            cx.fill(d, 3.0, INK);
        }
    }
    for (m, x, chip) in chips {
        let (sky, label) = &m.today;
        let (border, color) = if m.selected {
            (2.0, ACCENT)
        } else {
            (0.5, BORDER)
        };
        cx.fill_bordered(chip, 13.0, Color::WHITE, border, color);
        if let Some(sky) = sky {
            let r = Rect::new(chip.x + 6.0, chip.y + 4.0, 18.0, 18.0);
            cx.shape(&icons.sky[*sky as usize], r, INK);
        }
        cx.text(
            label,
            Font::BOLD,
            13.0,
            INK,
            None,
            (chip.x + 26.0, chip.y + 5.0),
        );
        if let Some(name) = m.name {
            let nw = cx.measure(name, Font::DEFAULT, 11.0, None).width;
            cx.text(
                name,
                Font::DEFAULT,
                11.0,
                INK,
                None,
                (x - nw / 2.0, chip.y + chip.h),
            );
        }
    }
}

fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// The wind: an arrow the way it blows, none when calm, and its speed in
/// km/h, followed by `unit`, in a column `width` wide.
fn wind_cells(w: forecast::Wind, icons: &Icons, unit: &str, width: f32) -> [Element; 3] {
    let to = w.from.map_or(0, |p| (p as usize + 4) % 8);
    let arrow = MUTED.alpha(if w.from.is_some() { 1.0 } else { 0.0 });
    [
        icon(&icons.wind).size(14.0).color(MUTED).into(),
        icon(&icons.arrows[to]).size(14.0).color(arrow).into(),
        // Fixed, so a shorter speed doesn't shift the columns before it.
        text(format!("{:.0}{unit}", w.kmh))
            .size(13.0)
            .color(MUTED)
            .width(width)
            .into(),
    ]
}

/// Whole degrees, without "-0".
fn deg(t: f32) -> f32 {
    t.round() + 0.0
}

/// A row of the cities table: name, today's sky, high and low, rain.
fn city_row(name: &str, f: &Option<forecast::Forecast>, icons: &Icons) -> Element {
    let mut cells: Vec<Element> = vec![text(name).color(INK).grow(1.0).into()];
    match f {
        None => cells.push(text("…").color(MUTED).into()),
        Some(Err(_)) => cells.push(text("–").color(MUTED).into()),
        Some(Ok(days)) => {
            if let Some(d) = days.first() {
                cells.extend(day_cells(d, icons));
            }
        }
    }
    row(cells)
        .gap(10.0)
        .align(AlignItems::CENTER)
        .fill_width()
        .into()
}

/// A city's days, each opening its hours.
fn days(
    f: &Option<forecast::Forecast>,
    open: Option<i64>,
    icons: &Icons,
    h: Handle<Ui>,
    name: &'static str,
    words: &'static Words,
) -> Vec<Element> {
    let days = match f {
        None => return vec![text(words.loading).color(MUTED).into()],
        Some(Err(e)) => return vec![text(e).size(13.0).color(ERROR).into()],
        Some(Ok(days)) => days,
    };
    let hourly = STORE.hourly.lock().unwrap().get(name).cloned();
    let mut out = Vec::new();
    for d in days {
        let (day, is_open) = (d.day, open == Some(d.day));
        let mut cells = vec![text(day_name(d.day, words)).color(INK).width(64.0).into()];
        cells.extend(day_cells(d, icons));
        if let Some(w) = d.wind {
            cells.extend(wind_cells(w, icons, "", 28.0));
        }
        let r = row(cells).gap(8.0).align(AlignItems::CENTER).fill_width();
        let b = button(r)
            .flat()
            .fill_width()
            .color(if is_open { TINT } else { Color::WHITE });
        out.push(
            b.on_click(move |cx| {
                cx.update(h, move |u: &mut Ui| {
                    u.day = if is_open { None } else { Some(day) }
                })
            })
            .into(),
        );
        if is_open {
            out.push(hours(hourly.as_ref(), day, icons, words));
        }
    }
    out
}

fn day_name(day: i64, words: &'static Words) -> &'static str {
    let today = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        / 86_400;
    match day - today as i64 {
        0 => words.today,
        1 => words.tomorrow,
        _ => words.weekdays[forecast::weekday(day)],
    }
}

/// Sky, high and low, and rain of a day.
fn day_cells(d: &Day, icons: &Icons) -> [Element; 4] {
    [
        icon(&icons.sky[d.sky as usize])
            .size(22.0)
            .color(INK)
            .into(),
        text(format!("{}° / {}°", deg(d.max), deg(d.min)))
            .color(INK)
            .width(80.0)
            .into(),
        icon(&icons.droplets).size(14.0).color(ACCENT).into(),
        text(&d.rain).size(13.0).color(MUTED).width(50.0).into(),
    ]
}

/// The hours of `day`, or why there are none.
fn hours(hourly: Option<&forecast::Hourly>, day: i64, icons: &Icons, words: &Words) -> Element {
    let note = |s: &str, c: Color| -> Vec<Element> { vec![text(s).size(13.0).color(c).into()] };
    let rows: Vec<Element> = match hourly {
        None => note(words.loading_hours, MUTED),
        Some(Err(e)) => note(e, ERROR),
        Some(Ok(hours)) => {
            let rows: Vec<Element> = (hours.iter().filter(|h| h.day == day))
                .map(|h| {
                    // A dry hour keeps the column's space, empty.
                    let wet = h.rain >= 0.1;
                    let mm = if wet {
                        format!("{:.1} mm", h.rain)
                    } else {
                        String::new()
                    };
                    let rain = text(mm).size(13.0).color(ACCENT).width(52.0);
                    let drop = ACCENT.alpha(if wet { 1.0 } else { 0.0 });
                    let mut cells = vec![
                        text(format!("{:02}:00", h.hour))
                            .size(13.0)
                            .color(MUTED)
                            .width(48.0)
                            .into(),
                        icon(&icons.sky[h.sky as usize])
                            .size(18.0)
                            .color(INK)
                            .into(),
                        text(format!("{}°", deg(h.temp)))
                            .size(13.0)
                            .grow(1.0)
                            .into(),
                        icon(&icons.droplets).size(13.0).color(drop).into(),
                        rain.into(),
                    ];
                    if let Some(c) = h.chance {
                        let c = text(format!("{c:.0} %")).size(13.0).color(MUTED);
                        cells.push(c.width(40.0).into());
                    }
                    if let Some(w) = h.wind {
                        cells.extend(wind_cells(w, icons, " km/h", 56.0));
                    }
                    row(cells).gap(8.0).align(AlignItems::CENTER).into()
                })
                .collect();
            if rows.is_empty() {
                note(words.no_hours, MUTED)
            } else {
                rows
            }
        }
    };
    column(rows)
        .gap(2.0)
        .padding_xy(12.0, 8.0)
        .bg(TINT)
        .radius(8.0)
        .into()
}

/// The app, with its forecasts already loading.
pub fn app() -> Meteo {
    forecast::start(COUNTRIES[0].cities, COUNTRIES[1].cities);
    Meteo {
        places: Rc::new([place(&COUNTRIES[0]), place(&COUNTRIES[1])]),
        icons: Rc::new(icons()),
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android: dfui::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
    dfui::run_android(app(), android);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_region_has_one_capital_and_a_border() {
        for c in &COUNTRIES {
            let geo: Value = serde_json::from_str(c.regions).unwrap();
            let names: Vec<&str> = (geo["features"].as_array().unwrap().iter())
                .map(|f| f["properties"]["name"].as_str().unwrap())
                .collect();
            for n in &names {
                let capitals = c.cities.iter().filter(|x| x.region == *n && x.capital);
                assert_eq!(capitals.count(), 1, "{n}");
            }
            for city in c.cities {
                assert!(names.contains(&city.region), "{}", city.name);
            }
        }
    }
}
