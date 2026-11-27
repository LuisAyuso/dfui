//! Daily and hourly forecasts: AEMET OpenData for Spain, GeoSphere Austria for Austria.
//! Fetched on worker threads into [`STORE`], which wakes the UI.

use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// The day's strongest wind.
    pub wind: Option<Wind>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wind {
    pub kmh: f32,
    /// Compass point it blows from, 0 north to 7 north-west, clockwise;
    /// none when calm.
    pub from: Option<u8>,
}

/// AEMET's compass points, Spanish: north is N, west is O.
fn aemet_wind(dir: &Value, kmh: &Value) -> Option<Wind> {
    // A single city's hourly forecast gives each as a one-item list.
    let one = |v: &Value| match v {
        Value::Array(a) => a.first().cloned().unwrap_or_default(),
        v => v.clone(),
    };
    let points = ["N", "NE", "E", "SE", "S", "SO", "O", "NO"];
    let dir = one(dir);
    Some(Wind {
        kmh: num(&one(kmh))?,
        from: points.iter().position(|p| dir == *p).map(|i| i as u8),
    })
}

/// Wind from its eastward and northward components, m/s.
fn uv_wind(u: f32, v: f32) -> Wind {
    let kmh = u.hypot(v) * 3.6;
    let from = (-u).atan2(-v).to_degrees().rem_euclid(360.0);
    Wind {
        kmh,
        from: (kmh >= 2.0).then(|| ((from / 45.0).round() as u8) % 8),
    }
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
    /// Chance of precipitation, %, where the source gives one.
    pub chance: Option<f32>,
    pub wind: Option<Wind>,
}

pub type Hourly = Result<Vec<Hour>, String>;

/// One place's days and hours.
type Series = (Vec<Day>, Vec<Hour>);

pub struct Store {
    pub notifier: Notifier,
    /// By city name; a city missing is still loading.
    pub data: Mutex<HashMap<&'static str, Forecast>>,
    pub hourly: Mutex<HashMap<&'static str, Hourly>>,
    /// AEMET is refusing requests for now; they're retried each minute.
    pub throttled: AtomicBool,
}

pub static STORE: LazyLock<Store> = LazyLock::new(|| Store {
    notifier: Notifier::new(),
    data: Mutex::default(),
    hourly: Mutex::default(),
    throttled: AtomicBool::new(false),
});

/// Stores `v` for `city` unless it's an error and the city already has data.
fn keep<T>(
    map: &Mutex<HashMap<&'static str, Result<T, String>>>,
    city: &'static str,
    v: Result<T, String>,
) {
    let mut map = map.lock().unwrap();
    if v.is_ok() || !map.get(city).is_some_and(Result::is_ok) {
        map.insert(city, v);
    }
    drop(map);
    STORE.notifier.notify();
}

fn put(city: &'static str, f: Forecast) {
    keep(&STORE.data, city, f);
}

fn put_hours(city: &'static str, h: Hourly) {
    keep(&STORE.hourly, city, h);
}

/// Loads each country's cities: Madrid on its own, then all of
/// Spain's municipalities from AEMET's daily and hourly batches (megabytes,
/// slow to come); Austria from one GeoSphere request.
pub fn start(spain: &'static [City], austria: &'static [City]) {
    std::thread::spawn(move || {
        let first = spain
            .iter()
            .find(|c| c.name == "Madrid")
            .unwrap_or(&spain[0]);
        let one = |kind| {
            let json = aemet_get(&format!("{kind}/{}", first.aemet))?;
            batch_shape(&String::from_utf8_lossy(&json))
        };
        put(first.name, one("diaria").and_then(|s| parse_aemet(&s)));
        put_hours(
            first.name,
            one("horaria").and_then(|s| parse_aemet_hourly(&s)),
        );
        let days = aemet_batch("diaria", "localidad_", spain);
        for (c, f) in spain.iter().zip(days) {
            put(c.name, f.and_then(|s| parse_aemet(&s)));
        }
        let hours = aemet_batch("horaria", "localidad_h_", spain);
        for (c, h) in spain.iter().zip(hours) {
            put_hours(c.name, h.and_then(|s| parse_aemet_hourly(&s)));
        }
    });
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
            put_hours(c.name, hours);
            put(c.name, days);
        }
    });
}

fn get_bytes(url: &str) -> Result<Vec<u8>, ureq::Error> {
    ureq::get(url)
        .call()?
        .body_mut()
        .with_config()
        .limit(64 << 20)
        .read_to_vec()
}

fn get(url: &str) -> Result<String, ureq::Error> {
    Ok(String::from_utf8_lossy(&get_bytes(url)?).into_owned())
}

/// Shown on the Spanish tab without a key, so in Spanish.
const NO_KEY: &str = "Falta AEMET_API_KEY (opendata.aemet.es)";

/// AEMET's `prediccion/especifica/municipio/{kind}/todos`, a .tar.gz with a
/// `{prefix}{INE code}.json` per municipality, as each city's JSON.
fn aemet_batch(kind: &str, prefix: &str, cities: &[City]) -> Vec<Result<String, String>> {
    let files = aemet_get(&format!("{kind}/todos")).and_then(|gz| {
        let names: Vec<String> = cities
            .iter()
            .map(|c| format!("{prefix}{}.json", c.aemet))
            .collect();
        untar(flate2::read::GzDecoder::new(gz.as_slice()), &names).map_err(|e| e.to_string())
    });
    match files {
        Ok(mut files) => cities
            .iter()
            .map(|c| {
                files
                    .remove(&format!("{prefix}{}.json", c.aemet))
                    .ok_or_else(|| format!("{} no está en AEMET", c.aemet))
            })
            .collect(),
        Err(e) => cities.iter().map(|_| Err(e.clone())).collect(),
    }
}

/// The files in `tar` named in `wanted`, skipping the rest unread.
fn untar(mut tar: impl Read, wanted: &[String]) -> std::io::Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    let mut header = [0; 512];
    while tar.read_exact(&mut header).is_ok() && header[0] != 0 {
        let field = |r: std::ops::Range<usize>| {
            let f = &header[r];
            String::from_utf8_lossy(&f[..f.iter().position(|&b| b == 0).unwrap_or(f.len())])
                .into_owned()
        };
        let name = field(0..100);
        let size = u64::from_str_radix(field(124..136).trim(), 8).unwrap_or(0);
        let padded = size.div_ceil(512) * 512;
        let mut body = (&mut tar).take(padded);
        if wanted.contains(&name) {
            let mut data = vec![0; padded as usize];
            body.read_exact(&mut data)?;
            data.truncate(size as usize);
            out.insert(name, String::from_utf8_lossy(&data).into_owned());
        } else {
            std::io::copy(&mut body, &mut std::io::sink())?;
        }
    }
    Ok(out)
}

/// AEMET's `prediccion/especifica/municipio/{path}`. It answers with a link
/// to the data; both requests count to its limit.
fn aemet_get(path: &str) -> Result<Vec<u8>, String> {
    let key = std::env::var("AEMET_API_KEY")
        .ok()
        .or(option_env!("AEMET_KEY_FILE").map(str::to_owned))
        .ok_or(NO_KEY)?;
    let url = format!(
        "https://opendata.aemet.es/opendata/api/prediccion/especifica/municipio/{path}?api_key={key}"
    );
    let mut dropped = 0;
    loop {
        let r = get(&url).and_then(|meta| {
            let meta: Value = serde_json::from_str(&meta).unwrap_or_default();
            get_bytes(meta["datos"].as_str().unwrap_or_default())
        });
        let throttled = matches!(r, Err(ureq::Error::StatusCode(429)));
        if STORE.throttled.swap(throttled, Ordering::Relaxed) != throttled {
            STORE.notifier.notify();
        }
        use ureq::Error::{ConnectionFailed, HostNotFound, Io, Timeout};
        match r {
            // AEMET's limit is per minute and it says to try again the next one.
            Err(_) if throttled => std::thread::sleep(Duration::from_secs(61)),
            // The batches are megabytes; a phone's connection may drop mid-way.
            Err(Io(_) | Timeout(_) | ConnectionFailed | HostNotFound) if dropped < 4 => {
                dropped += 1;
                std::thread::sleep(Duration::from_secs(2 << dropped));
            }
            Err(e @ (Io(_) | Timeout(_) | ConnectionFailed | HostNotFound)) => {
                return Err(format!("Sin conexión con AEMET ({e})"));
            }
            r => return r.map_err(|e| e.to_string()),
        }
    }
}

/// A number, which AEMET's batches give as a string ("Ip" is a trace of rain).
fn num(v: &Value) -> Option<f32> {
    v.as_f64()
        .map(|x| x as f32)
        .or_else(|| v.as_str()?.parse().ok())
}

/// A single municipality's forecast in the batches' shape: under `root`,
/// keys in snake case, `valor` for `value`.
fn batch_shape(json: &str) -> Result<String, String> {
    fn rename(v: Value) -> Value {
        match v {
            Value::Array(a) => Value::Array(a.into_iter().map(rename).collect()),
            Value::Object(o) => (o.into_iter())
                .map(|(k, v)| {
                    let k = match k.as_str() {
                        "value" => "valor".to_owned(),
                        "vientoAndRachaMax" => "viento".to_owned(),
                        _ => k.chars().fold(String::new(), |mut s, c| {
                            if c.is_uppercase() {
                                s.push('_');
                            }
                            s.extend(c.to_lowercase());
                            s
                        }),
                    };
                    (k, rename(v))
                })
                .collect(),
            v => v,
        }
    }
    let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "root": rename(v[0].clone()) }).to_string())
}

/// Series `k` of `d`; AEMET's batches, converted from XML, give a one-entry
/// series as the entry itself.
fn list<'a>(d: &'a Value, k: &str) -> &'a [Value] {
    match &d[k] {
        Value::Array(a) => a,
        Value::Null => &[],
        v => std::slice::from_ref(v),
    }
}

/// The `valor` of the entry in series `k` for `periodo`.
fn at<'a>(d: &'a Value, k: &str, periodo: &str) -> Option<&'a Value> {
    Some(&list(d, k).iter().find(|e| e["periodo"] == periodo)?["valor"])
}

fn parse_aemet(json: &str) -> Forecast {
    let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let days = v["root"]["prediccion"]["dia"]
        .as_array()
        .ok_or("no forecast")?;
    Ok(days
        .iter()
        .filter_map(|d| {
            let sky = list(d, "estado_cielo")
                .iter()
                .filter(|s| s["valor"].as_str().is_some_and(|v| !v.is_empty()));
            // The whole day if given, else the first period that has a value.
            let sky = sky
                .clone()
                .find(|s| s["periodo"] == "00-24")
                .or(sky.clone().next());
            let rain = list(d, "prob_precipitacion")
                .iter()
                .filter_map(|p| num(&p["valor"]));
            Some(Day {
                day: days_from_iso(d["fecha"].as_str()?)?,
                sky: aemet_sky(sky.and_then(|s| s["valor"].as_str()).unwrap_or("")),
                min: num(&d["temperatura"]["minima"])?,
                max: num(&d["temperatura"]["maxima"])?,
                rain: format!("{:.0} %", rain.fold(0.0, f32::max)),
                wind: (list(d, "viento").iter())
                    .filter_map(|w| aemet_wind(&w["direccion"], &w["velocidad"]))
                    .max_by(|a, b| a.kmh.total_cmp(&b.kmh)),
            })
        })
        .collect())
}

/// The chance of precipitation for `hour` from AEMET's blocks of hours,
/// periods like "0713" and "1901" (past midnight).
fn chance(d: &Value, hour: &str) -> Option<f32> {
    let h: u32 = hour.parse().ok()?;
    list(d, "prob_precipitacion").iter().find_map(|p| {
        let period = p["periodo"].as_str()?;
        let (from, to) = (
            period.get(..2)?.parse().ok()?,
            period.get(2..)?.parse().ok()?,
        );
        let inside = if from < to {
            (from..to).contains(&h)
        } else {
            h >= from || h < to
        };
        inside.then(|| num(&p["valor"])).flatten()
    })
}

/// AEMET's hourly forecast: per day, series of `{periodo, valor}` where the
/// period is the hour.
fn parse_aemet_hourly(json: &str) -> Hourly {
    let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let days = v["root"]["prediccion"]["dia"]
        .as_array()
        .ok_or("no forecast")?;
    let mut out = Vec::new();
    for d in days {
        let Some(day) = d["fecha"].as_str().and_then(days_from_iso) else {
            continue;
        };
        for t in list(d, "temperatura") {
            let (Some(hour), Some(temp)) = (t["periodo"].as_str(), num(&t["valor"])) else {
                continue;
            };
            let mm = |k| at(d, k, hour).and_then(num).unwrap_or(0.0);
            out.push(Hour {
                day,
                hour: hour.parse().unwrap_or(0),
                sky: aemet_sky(
                    at(d, "estado_cielo", hour)
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                ),
                temp,
                rain: mm("precipitacion") + mm("nieve"),
                chance: chance(d, hour),
                wind: (list(d, "viento").iter())
                    .find(|w| w["periodo"] == hour && !w["direccion"].is_null())
                    .and_then(|w| aemet_wind(&w["direccion"], &w["velocidad"])),
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

const GEOSPHERE: &str = "https://dataset.api.hub.geosphere.at/v1/timeseries/forecast/nwp-v2-1h-1km?parameters=2t,tcc,tp,sf,cape,10u,10v";

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
            let (u, v) = (series("10u"), series("10v"));
            let wind = |k: usize| uv_wind(u[k], v[k]);
            let hours: Vec<Hour> = (0..local.len())
                .filter_map(|k| {
                    let (day, hour) = local[k]?;
                    Some(Hour {
                        day,
                        hour,
                        sky: sky(tcc[k], tp[k], sf[k], cape[k] > STORM_CAPE, 0.1),
                        temp: t[k],
                        rain: tp[k],
                        chance: None,
                        wind: Some(wind(k)),
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
                    wind: (chunk.iter().map(|&k| wind(k))).max_by(|a, b| a.kmh.total_cmp(&b.kmh)),
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
        let json = r#"{"root":{"prediccion":{"dia":[
            {"fecha":"2026-11-27","temperatura":{"maxima":"14","minima":"3"},
             "estado_cielo":[{"periodo":"00-24","descripcion":""},{"valor":"25","periodo":"00-12"}],
             "prob_precipitacion":[{"periodo":"00-24"},{"valor":"40","periodo":"00-12"},{"valor":"85","periodo":"12-24"}]},
            {"fecha":"2026-11-28","temperatura":{"maxima":"16","minima":"5"},
             "estado_cielo":{"valor":"11n"},"prob_precipitacion":{"valor":"0"}}]}}}"#;
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
        let json = r#"{"root":{"prediccion":{"dia":[{"fecha":"2026-11-27",
            "temperatura":[{"valor":"5","periodo":"07"},{"valor":"9","periodo":"08"}],
            "estado_cielo":[{"valor":"16n","periodo":"07"},{"valor":"25","periodo":"08"}],
            "precipitacion":[{"valor":"Ip","periodo":"07"},{"valor":"1.2","periodo":"08"}],
            "nieve":[{"valor":"0","periodo":"07"},{"valor":"0","periodo":"08"}],
            "viento":[{"periodo":"07","direccion":"C","velocidad":"0"},{"periodo":"08","direccion":"SO","velocidad":"18"}],
            "prob_precipitacion":[{"periodo":"0107","valor":"10"},{"periodo":"0713","valor":"60"}]}]}}}"#;
        let h = parse_aemet_hourly(json).unwrap();
        assert_eq!(h.len(), 2);
        assert_eq!(
            (h[0].hour, h[0].sky, h[0].temp, h[0].rain),
            (7, Sky::Cloudy, 5.0, 0.0)
        );
        assert_eq!((h[1].sky, h[1].rain), (Sky::Rain, 1.2));
        let calm = Wind {
            kmh: 0.0,
            from: None,
        };
        let south_west = Wind {
            kmh: 18.0,
            from: Some(5),
        };
        assert_eq!((h[0].wind, h[1].wind), (Some(calm), Some(south_west)));
        assert_eq!((h[0].chance, h[1].chance), (Some(60.0), Some(60.0)));
        let late = serde_json::json!({"prob_precipitacion":[{"periodo":"1901","valor":"30"}]});
        assert_eq!(
            (
                chance(&late, "23"),
                chance(&late, "00"),
                chance(&late, "12")
            ),
            (Some(30.0), Some(30.0), None)
        );
    }

    #[test]
    fn single_city_shape() {
        let json = r#"[{"prediccion":{"dia":[{"fecha":"2026-11-27T00:00:00",
            "temperatura":{"maxima":14,"minima":3},
            "estadoCielo":[{"value":"25","periodo":"00-24"}],
            "probPrecipitacion":[{"value":85,"periodo":"00-24"}],
            "viento":[{"direccion":"N","velocidad":10,"periodo":"00-12"},{"direccion":"O","velocidad":25,"periodo":"12-24"}]}]}}]"#;
        let days = parse_aemet(&batch_shape(json).unwrap()).unwrap();
        assert_eq!(
            (days[0].sky, days[0].min, days[0].rain.as_str()),
            (Sky::Rain, 3.0, "85 %")
        );
        assert_eq!(
            days[0].wind,
            Some(Wind {
                kmh: 25.0,
                from: Some(6)
            })
        );
        let hourly = r#"[{"prediccion":{"dia":[{"fecha":"2026-11-27T00:00:00",
            "temperatura":[{"value":"5","periodo":"07"}],
            "vientoAndRachaMax":[{"direccion":["NE"],"velocidad":["12"],"periodo":"07"},{"value":"30","periodo":"07"}]}]}}]"#;
        let h = parse_aemet_hourly(&batch_shape(hourly).unwrap()).unwrap();
        assert_eq!(
            h[0].wind,
            Some(Wind {
                kmh: 12.0,
                from: Some(1)
            })
        );
    }

    #[test]
    fn wind_from_components() {
        // Blowing towards the east is a west wind; 10 m/s is 36 km/h.
        let w = uv_wind(10.0, 0.0);
        assert_eq!((w.kmh.round(), w.from), (36.0, Some(6)));
        assert_eq!(uv_wind(0.0, -5.0).from, Some(0));
        assert_eq!(uv_wind(0.1, 0.1).from, None);
    }

    #[test]
    fn untars_only_the_wanted_files() {
        let mut tar = Vec::new();
        for (name, body) in [("a.json", "skipped"), ("b.json", "{}")] {
            let mut h = [0u8; 512];
            h[..name.len()].copy_from_slice(name.as_bytes());
            h[124..135].copy_from_slice(format!("{:011o}", body.len()).as_bytes());
            tar.extend(h);
            tar.extend(body.bytes().chain(std::iter::repeat(0)).take(512));
        }
        tar.extend([0; 1024]);
        let files = untar(tar.as_slice(), &["b.json".to_owned()]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files["b.json"], "{}");
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
              "cape":{{"data":[0,0,0,0,0,0,0,0,0,0,0,0,0,0]}},
              "10u":{{"data":[0,0,0,0,0,0,0,0,0,0,0,0,0,0]}},
              "10v":{{"data":[1,1,1,5,1,1,1,0,0,0,0,0,0,0]}}}}}}}}]}}"#,
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
        // The day's strongest, 5 m/s from the south.
        assert_eq!(
            days[0].wind,
            Some(Wind {
                kmh: 18.0,
                from: Some(4)
            })
        );
    }
}
