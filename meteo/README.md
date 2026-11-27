# meteo

Daily forecasts for Spain and Austria on a map of their regions, built with
dfui and dfui-map.

```sh
AEMET_API_KEY=... cargo run -p meteo --release
```

Tap a city for its days. Spain needs a free AEMET OpenData key
(<https://opendata.aemet.es/centrodedescargas/altaUsuario>); without it the
Spanish cities show the error and Austria still works.

## Data

- Spain: AEMET OpenData, daily forecast per municipality (INE code), ~7 days.
  The cities are fetched one by one to stay under AEMET's rate limit.
- Austria: GeoSphere Austria Dataset API, `nwp-v2-1h-1km`, no key; ~60 hourly
  steps folded into days. GeoSphere's weather symbol codes aren't published,
  so the sky comes from cloud cover, precipitation, snowfall and CAPE.
- Regions: Eurostat GISCO NUTS 2 (2021, 1:3M), © EuroGeographics, kept to ES
  and AT in `assets/*.geojson`. The Canary Islands are moved next to the
  peninsula.
- Icons: [Lucide](https://lucide.dev) 1.82.0 (ISC), converted with
  `cargo run -p svg2path -- <svg dir> meteo/assets/icons/`.

Map units are km, not dfui-map's metres: its tiles are sized for cities.
