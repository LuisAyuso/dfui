//! Daily forecasts: AEMET OpenData for Spain, GeoSphere Austria for Austria.
//! Fetched on worker threads into [`STORE`], which wakes the UI.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use dfui::Notifier;
use serde_json::Value;

use crate::City;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sky {
    Clear,
    PartlyCloudy,
    Cloudy,
    Fog,
    Rain,
    Snow,
    Storm,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    /// Days since 1970-01-01.
    pub day: i64,
    pub sky: Sky,
    pub min: f32,
    pub max: f32,
    /// Chance of precipitation (AEMET) or amount (GeoSphere), as shown.
    pub rain: String,
}

pub type Forecast = Result<Vec<Day>, String>;

pub struct Store {
    pub notifier: Notifier,
    /// By city name; a city missing is still loading.
    pub data: Mutex<HashMap<&'static str, Forecast>>,
}

pub static STORE: LazyLock<Store> = LazyLock::new(|| Store {
    notifier: Notifier::new(),
    data: Mutex::default(),
});

fn put(city: &'static str, f: Forecast) {
    STORE.data.lock().unwrap().insert(city, f);
    STORE.notifier.notify();
}

pub fn start(spain: &'static [City], austria: &'static [City]) {
    std::thread::spawn(move || aemet(spain));
    std::thread::spawn(move || {
        let r = geosphere(austria);
        for (i, c) in austria.iter().enumerate() {
            put(
                c.name,
                r.as_ref()
                    .map(|days| days.get(i).cloned().unwrap_or_default())
                    .map_err(Clone::clone),
            );
        }
    });
}

fn get(url: &str) -> Result<String, ureq::Error> {
    let bytes = ureq::get(url).call()?.body_mut().read_to_vec()?;
    // AEMET serves ISO-8859-15; close enough to Latin-1 for city names.
    Ok(String::from_utf8(bytes)
        .unwrap_or_else(|e| e.into_bytes().iter().map(|&b| b as char).collect()))
}

/// AEMET answers with a link to the data; both requests count to its limit.
fn aemet(cities: &'static [City]) {
    let Ok(key) = std::env::var("AEMET_API_KEY") else {
        for c in cities {
            put(c.name, Err("Set AEMET_API_KEY (opendata.aemet.es)".into()));
        }
        return;
    };
    for c in cities {
        let url = format!(
            "https://opendata.aemet.es/opendata/api/prediccion/especifica/municipio/diaria/{}?api_key={key}",
            c.aemet
        );
        let mut wait = Duration::from_secs(5);
        let f = loop {
            let r = get(&url).and_then(|meta| {
                let meta: Value = serde_json::from_str(&meta).unwrap_or_default();
                get(meta["datos"].as_str().unwrap_or_default())
            });
            match r {
                Err(ureq::Error::StatusCode(429)) if wait < Duration::from_secs(60) => {
                    std::thread::sleep(wait);
                    wait *= 2;
                }
                r => break r.map_err(|e| e.to_string()).and_then(|s| parse_aemet(&s)),
            }
        };
        put(c.name, f);
        // ponytail: fixed pacing under AEMET's per-minute limit; read its rate headers if it still throttles.
        std::thread::sleep(Duration::from_millis(1500));
    }
}

fn parse_aemet(json: &str) -> Forecast {
    let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let days = v[0]["prediccion"]["dia"].as_array().ok_or("no forecast")?;
    Ok(days
        .iter()
        .filter_map(|d| {
            let sky = d["estadoCielo"].as_array()?;
            let sky = sky
                .iter()
                .filter(|s| s["value"].as_str().is_some_and(|v| !v.is_empty()));
            // The whole day if given, else the first period that has a value.
            let sky = sky
                .clone()
                .find(|s| s["periodo"] == "00-24")
                .or(sky.clone().next());
            let rain = d["probPrecipitacion"]
                .as_array()?
                .iter()
                .filter_map(|p| p["value"].as_f64());
            Some(Day {
                day: days_from_iso(d["fecha"].as_str()?)?,
                sky: aemet_sky(sky.and_then(|s| s["value"].as_str()).unwrap_or("")),
                min: d["temperatura"]["minima"].as_f64()? as f32,
                max: d["temperatura"]["maxima"].as_f64()? as f32,
                rain: format!("{:.0} %", rain.fold(0.0, f64::max)),
            })
        })
        .collect())
}

/// AEMET's "estado del cielo" codes; an "n" suffix marks night.
fn aemet_sky(code: &str) -> Sky {
    match code.trim_end_matches('n').parse::<u32>().unwrap_or(0) {
        11 => Sky::Clear,
        12 | 13 | 17 => Sky::PartlyCloudy,
        81..=83 => Sky::Fog,
        23..=26 | 43..=46 => Sky::Rain,
        33..=36 | 71..=74 => Sky::Snow,
        51..=54 | 61..=64 => Sky::Storm,
        _ => Sky::Cloudy,
    }
}

const GEOSPHERE: &str = "https://dataset.api.hub.geosphere.at/v1/timeseries/forecast/nwp-v2-1h-1km?parameters=2t,tcc,tp,sf,cape";

/// One request for all cities: ~60 hourly steps, folded into days.
fn geosphere(cities: &[City]) -> Result<Vec<Vec<Day>>, String> {
    let mut url = GEOSPHERE.to_owned();
    for c in cities {
        url += &format!("&lat_lon={},{}", c.lat, c.lon);
    }
    parse_geosphere(&get(&url).map_err(|e| e.to_string())?)
}

fn parse_geosphere(json: &str) -> Result<Vec<Vec<Day>>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let times = v["timestamps"].as_array().ok_or("no timestamps")?;
    // ponytail: CET fixed at UTC+1; summer days shift by an hour.
    let day_of = |t: &Value| -> Option<i64> {
        let t = t.as_str()?;
        let hour: i64 = t.get(11..13)?.parse().ok()?;
        Some((days_from_iso(t)? * 24 + hour + 1).div_euclid(24))
    };
    let days: Vec<Option<i64>> = times.iter().map(day_of).collect();
    let features = v["features"].as_array().ok_or("no features")?;
    Ok(features
        .iter()
        .map(|f| {
            let p = &f["properties"]["parameters"];
            let series = |k: &str| -> Vec<f32> {
                let d = p[k]["data"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                d.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()
            };
            let (t, tcc, tp, sf, cape) = (
                series("2t"),
                series("tcc"),
                series("tp"),
                series("sf"),
                series("cape"),
            );
            let mut out: Vec<Day> = Vec::new();
            let mut i = 0;
            while i < days.len() {
                let d = days[i];
                let n = days[i..].iter().take_while(|&&x| x == d).count();
                let r = i..(i + n).min(t.len());
                i += n;
                // Days with only a few hours left in the run say little.
                let (Some(day), true) = (d, r.len() >= 6) else {
                    continue;
                };
                let sum = |s: &[f32]| s.get(r.clone()).map_or(0.0, |s| s.iter().sum::<f32>());
                let (rain, snow) = (sum(&tp), sum(&sf));
                let cloud = sum(&tcc) / r.len() as f32;
                let storm = cape
                    .get(r.clone())
                    .is_some_and(|c| c.iter().any(|&c| c > 800.0));
                let sky = match () {
                    _ if rain >= 1.0 && snow >= rain / 2.0 => Sky::Snow,
                    _ if rain >= 1.0 && storm => Sky::Storm,
                    _ if rain >= 1.0 => Sky::Rain,
                    _ if cloud < 20.0 => Sky::Clear,
                    _ if cloud < 70.0 => Sky::PartlyCloudy,
                    _ => Sky::Cloudy,
                };
                let temps = &t[r];
                out.push(Day {
                    day,
                    sky,
                    min: temps.iter().copied().fold(f32::INFINITY, f32::min),
                    max: temps.iter().copied().fold(f32::NEG_INFINITY, f32::max),
                    rain: format!("{rain:.1} mm"),
                });
            }
            out
        })
        .collect())
}

/// Days since 1970-01-01 of an ISO date ("2026-11-27…").
fn days_from_iso(s: &str) -> Option<i64> {
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    Some(era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468)
}

pub fn weekday(day: i64) -> &'static str {
    ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][day.rem_euclid(7) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(days_from_iso("1970-01-01"), Some(0));
        assert_eq!(
            weekday(days_from_iso("2026-11-27T00:00:00").unwrap()),
            "Fri"
        );
        assert_eq!(weekday(days_from_iso("2024-02-29").unwrap()), "Thu");
    }

    #[test]
    fn aemet() {
        let json = r#"[{"prediccion":{"dia":[
            {"fecha":"2026-11-27T00:00:00","temperatura":{"maxima":14,"minima":3},
             "estadoCielo":[{"value":"","periodo":"00-24"},{"value":"25","periodo":"00-12"}],
             "probPrecipitacion":[{"value":40,"periodo":"00-12"},{"value":85,"periodo":"12-24"}]},
            {"fecha":"2026-11-28T00:00:00","temperatura":{"maxima":16,"minima":5},
             "estadoCielo":[{"value":"11n"}],"probPrecipitacion":[{"value":0}]}]}}]"#;
        let days = parse_aemet(json).unwrap();
        assert_eq!(days.len(), 2);
        assert_eq!(
            (days[0].sky, days[0].max, days[0].rain.as_str()),
            (Sky::Rain, 14.0, "85 %")
        );
        assert_eq!(days[1].sky, Sky::Clear);
    }

    #[test]
    fn geosphere() {
        // 22:00 UTC is 23:00 CET, still the first day; the second has 7 h.
        let hours = (16..30).map(|h| format!("\"2026-11-{}T{:02}:00+00:00\"", 27 + h / 24, h % 24));
        let ts: Vec<String> = hours.collect();
        let json = format!(
            r#"{{"timestamps":[{}],"features":[{{"properties":{{"parameters":{{
              "2t":{{"data":[1,2,3,4,5,6,7,-1,-2,-3,-4,-5,-6,-7]}},
              "tcc":{{"data":[0,0,0,0,0,0,0,100,100,100,100,100,100,100]}},
              "tp":{{"data":[0,0,0,0,0,0,0,1,1,0,0,0,0,0]}},
              "sf":{{"data":[0,0,0,0,0,0,0,1,1,0,0,0,0,0]}},
              "cape":{{"data":[0,0,0,0,0,0,0,0,0,0,0,0,0,0]}}}}}}}}]}}"#,
            ts.join(",")
        );
        let days = &parse_geosphere(&json).unwrap()[0];
        assert_eq!(days.len(), 2);
        assert_eq!(
            (days[0].sky, days[0].min, days[0].max),
            (Sky::Clear, 1.0, 7.0)
        );
        assert_eq!((days[1].sky, days[1].rain.as_str()), (Sky::Snow, "2.0 mm"));
    }
}
