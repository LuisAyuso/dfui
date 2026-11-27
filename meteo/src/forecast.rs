//! Daily and hourly forecasts: AEMET OpenData for Spain, GeoSphere Austria for Austria.
//! Fetched on worker threads into [`STORE`], which wakes the UI.

use std::collections::{HashMap, VecDeque};
use std::sync::{Condvar, LazyLock, Mutex};
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

#[derive(Clone, Debug, PartialEq)]
pub struct Hour {
    pub day: i64,
    /// Local hour, 0–23.
    pub hour: i64,
    pub sky: Sky,
    pub temp: f32,
    /// Precipitation, mm.
    pub rain: f32,
}

pub type Hourly = Result<Vec<Hour>, String>;

/// One place's days and hours.
type Series = (Vec<Day>, Vec<Hour>);

pub struct Store {
    pub notifier: Notifier,
    /// By city name; a city missing is still loading.
    pub data: Mutex<HashMap<&'static str, Forecast>>,
    pub hourly: Mutex<HashMap<&'static str, Hourly>>,
}

pub static STORE: LazyLock<Store> = LazyLock::new(|| Store {
    notifier: Notifier::new(),
    data: Mutex::default(),
    hourly: Mutex::default(),
});

fn put(city: &'static str, f: Forecast) {
    STORE.data.lock().unwrap().insert(city, f);
    STORE.notifier.notify();
}

/// Loads Austria in one request and starts the AEMET queue with the
/// capitals' days; other Spanish cities wait for [`want`].
pub fn start(spain: &'static [City], austria: &'static [City]) {
    for c in spain.iter().filter(|c| c.capital).rev() {
        want(c, false);
    }
    std::thread::spawn(aemet_worker);
    std::thread::spawn(move || {
        let r = geosphere(austria);
        for (i, c) in austria.iter().enumerate() {
            let (days, hours) = match &r {
                Ok(all) => {
                    let (d, h) = all.get(i).cloned().unwrap_or_default();
                    (Ok(d), Ok(h))
                }
                Err(e) => (Err(e.clone()), Err(e.clone())),
            };
            STORE.hourly.lock().unwrap().insert(c.name, hours);
            put(c.name, days);
        }
    });
}

/// AEMET requests still to make: a city and whether it's the hourly one.
static QUEUE: Mutex<VecDeque<(&City, bool)>> = Mutex::new(VecDeque::new());
static QUEUED: Condvar = Condvar::new();

/// Queues `city`'s daily or hourly AEMET forecast ahead of what's waiting,
/// once: what the user just opened goes first.
pub fn want(city: &'static City, hourly: bool) {
    static ASKED: Mutex<Vec<(&str, bool)>> = Mutex::new(Vec::new());
    let mut asked = ASKED.lock().unwrap();
    if city.aemet.is_empty() || asked.contains(&(city.name, hourly)) {
        return;
    }
    asked.push((city.name, hourly));
    QUEUE.lock().unwrap().push_front((city, hourly));
    QUEUED.notify_one();
}

/// Makes the queued requests one at a time, paced for AEMET's rate limit.
fn aemet_worker() {
    loop {
        let (city, hourly) = {
            let mut q = QUEUE.lock().unwrap();
            loop {
                match q.pop_front() {
                    Some(job) => break job,
                    None => q = QUEUED.wait(q).unwrap(),
                }
            }
        };
        let r = aemet_get(if hourly { "horaria" } else { "diaria" }, city.aemet);
        let no_key = r.as_ref().is_err_and(|e| e == NO_KEY);
        if hourly {
            let h = r.and_then(|s| parse_aemet_hourly(&s));
            STORE.hourly.lock().unwrap().insert(city.name, h);
            STORE.notifier.notify();
        } else {
            put(city.name, r.and_then(|s| parse_aemet(&s)));
        }
        if !no_key {
            // ponytail: fixed pacing under AEMET's per-minute limit; read its rate headers if it still throttles.
            std::thread::sleep(Duration::from_millis(1500));
        }
    }
}

fn get(url: &str) -> Result<String, ureq::Error> {
    let bytes = ureq::get(url).call()?.body_mut().read_to_vec()?;
    // AEMET serves ISO-8859-15; close enough to Latin-1 for city names.
    Ok(String::from_utf8(bytes)
        .unwrap_or_else(|e| e.into_bytes().iter().map(|&b| b as char).collect()))
}

/// Shown on the Spanish tab without a key, so in Spanish.
const NO_KEY: &str = "Falta AEMET_API_KEY (opendata.aemet.es)";

/// AEMET's `prediccion/especifica/municipio/{kind}` for a municipality.
/// It answers with a link to the data; both requests count to its limit.
fn aemet_get(kind: &str, code: &str) -> Result<String, String> {
    let key = std::env::var("AEMET_API_KEY").map_err(|_| NO_KEY)?;
    let url = format!(
        "https://opendata.aemet.es/opendata/api/prediccion/especifica/municipio/{kind}/{code}?api_key={key}"
    );
    let mut wait = Duration::from_secs(5);
    loop {
        let r = get(&url).and_then(|meta| {
            let meta: Value = serde_json::from_str(&meta).unwrap_or_default();
            get(meta["datos"].as_str().unwrap_or_default())
        });
        match r {
            Err(ureq::Error::StatusCode(429)) if wait < Duration::from_secs(60) => {
                std::thread::sleep(wait);
                wait *= 2;
            }
            r => return r.map_err(|e| e.to_string()),
        }
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

/// AEMET's hourly forecast: per day, series of `{value, periodo}` where the
/// period is the hour and values are strings ("Ip" is a trace of rain).
fn parse_aemet_hourly(json: &str) -> Hourly {
    let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let days = v[0]["prediccion"]["dia"].as_array().ok_or("no forecast")?;
    let mut out = Vec::new();
    for d in days {
        let Some(day) = d["fecha"].as_str().and_then(days_from_iso) else {
            continue;
        };
        let at = |k: &str, hour: &str| -> Option<String> {
            let s = d[k].as_array()?.iter().find(|e| e["periodo"] == hour)?;
            s["value"].as_str().map(str::to_owned)
        };
        for t in d["temperatura"].as_array().into_iter().flatten() {
            let (Some(hour), Some(temp)) = (t["periodo"].as_str(), t["value"].as_str()) else {
                continue;
            };
            let num = |k| {
                at(k, hour)
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(0.0)
            };
            let (rain, snow) = (num("precipitacion"), num("nieve"));
            out.push(Hour {
                day,
                hour: hour.parse().unwrap_or(0),
                sky: aemet_sky(&at("estadoCielo", hour).unwrap_or_default()),
                temp: temp.parse().unwrap_or(f32::NAN),
                rain: rain + snow,
            });
        }
    }
    Ok(out)
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

/// One request for all cities: ~60 hourly steps, and the same folded into days.
fn geosphere(cities: &[City]) -> Result<Vec<Series>, String> {
    let mut url = GEOSPHERE.to_owned();
    for c in cities {
        url += &format!("&lat_lon={},{}", c.lat, c.lon);
    }
    parse_geosphere(&get(&url).map_err(|e| e.to_string())?)
}

/// The sky from cloud cover (%), precipitation and snowfall (mm), with
/// precipitation counting from `wet` mm.
fn sky(cloud: f32, rain: f32, snow: f32, storm: bool, wet: f32) -> Sky {
    match () {
        _ if rain >= wet && snow >= rain / 2.0 => Sky::Snow,
        _ if rain >= wet && storm => Sky::Storm,
        _ if rain >= wet => Sky::Rain,
        _ if cloud < 20.0 => Sky::Clear,
        _ if cloud < 70.0 => Sky::PartlyCloudy,
        _ => Sky::Cloudy,
    }
}

/// CAPE above which rain counts as a thunderstorm, J/kg.
const STORM_CAPE: f32 = 800.0;

fn parse_geosphere(json: &str) -> Result<Vec<Series>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let times = v["timestamps"].as_array().ok_or("no timestamps")?;
    // ponytail: CET fixed at UTC+1; summer hours are off by one.
    let local = |t: &Value| -> Option<(i64, i64)> {
        let t = t.as_str()?;
        let hour: i64 = t.get(11..13)?.parse().ok()?;
        let h = days_from_iso(t)? * 24 + hour + 1;
        Some((h.div_euclid(24), h.rem_euclid(24)))
    };
    let local: Vec<Option<(i64, i64)>> = times.iter().map(local).collect();
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
                let d = d.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32);
                d.chain(std::iter::repeat(0.0)).take(local.len()).collect()
            };
            let (t, tcc, tp, sf, cape) = (
                series("2t"),
                series("tcc"),
                series("tp"),
                series("sf"),
                series("cape"),
            );
            let hours: Vec<Hour> = (0..local.len())
                .filter_map(|k| {
                    let (day, hour) = local[k]?;
                    Some(Hour {
                        day,
                        hour,
                        sky: sky(tcc[k], tp[k], sf[k], cape[k] > STORM_CAPE, 0.1),
                        temp: t[k],
                        rain: tp[k],
                    })
                })
                .collect();
            let known: Vec<usize> = (0..local.len()).filter(|&k| local[k].is_some()).collect();
            let day_of = |k: usize| local[k].map(|l| l.0);
            let mut days: Vec<Day> = Vec::new();
            for chunk in known.chunk_by(|&a, &b| day_of(a) == day_of(b)) {
                // Days with only a few hours left in the run say little.
                let (Some(day), true) = (day_of(chunk[0]), chunk.len() >= 6) else {
                    continue;
                };
                let sum = |s: &[f32]| chunk.iter().map(|&k| s[k]).sum::<f32>();
                let (rain, snow) = (sum(&tp), sum(&sf));
                let storm = chunk.iter().any(|&k| cape[k] > STORM_CAPE);
                let temps = chunk.iter().map(|&k| t[k]);
                days.push(Day {
                    day,
                    sky: sky(sum(&tcc) / chunk.len() as f32, rain, snow, storm, 1.0),
                    min: temps.clone().fold(f32::INFINITY, f32::min),
                    max: temps.fold(f32::NEG_INFINITY, f32::max),
                    rain: format!("{rain:.1} mm"),
                });
            }
            (days, hours)
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

/// Day of the week, Monday 0; 1970-01-01 was a Thursday.
pub fn weekday(day: i64) -> usize {
    (day + 3).rem_euclid(7) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(days_from_iso("1970-01-01"), Some(0));
        assert_eq!(weekday(days_from_iso("2026-11-27T00:00:00").unwrap()), 4);
        assert_eq!(weekday(days_from_iso("2024-02-29").unwrap()), 3);
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
    fn aemet_hourly() {
        let json = r#"[{"prediccion":{"dia":[{"fecha":"2026-11-27T00:00:00",
            "temperatura":[{"value":"5","periodo":"07"},{"value":"9","periodo":"08"}],
            "estadoCielo":[{"value":"16n","periodo":"07"},{"value":"25","periodo":"08"}],
            "precipitacion":[{"value":"Ip","periodo":"07"},{"value":"1.2","periodo":"08"}],
            "nieve":[{"value":"0","periodo":"07"},{"value":"0","periodo":"08"}]}]}}]"#;
        let h = parse_aemet_hourly(json).unwrap();
        assert_eq!(h.len(), 2);
        assert_eq!(
            (h[0].hour, h[0].sky, h[0].temp, h[0].rain),
            (7, Sky::Cloudy, 5.0, 0.0)
        );
        assert_eq!((h[1].sky, h[1].rain), (Sky::Rain, 1.2));
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
        let (days, hours) = &parse_geosphere(&json).unwrap()[0];
        assert_eq!(days.len(), 2);
        assert_eq!(hours.len(), 14);
        assert_eq!(
            (hours[0].hour, hours[7].hour, hours[7].day),
            (17, 0, days[1].day)
        );
        assert_eq!((hours[7].sky, hours[7].rain), (Sky::Snow, 1.0));
        assert_eq!(
            (days[0].sky, days[0].min, days[0].max),
            (Sky::Clear, 1.0, 7.0)
        );
        assert_eq!((days[1].sky, days[1].rain.as_str()), (Sky::Snow, "2.0 mm"));
    }
}
