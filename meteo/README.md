# meteo

Daily and hourly forecasts for Spain and Austria on a map of their regions,
built with dfui and dfui-map. Phone-shaped: the map takes the top third, the
forecast table the rest.

```sh
AEMET_API_KEY=... cargo run -p meteo --release
```

The country shows its regional capitals; tap one (on the map or in the
table) and the map zooms to its region and shows the region's other cities.
Tap a city for its days, a day for its hours, and the back arrow to go up. Spain needs a free AEMET OpenData key
(<https://opendata.aemet.es/centrodedescargas/altaUsuario>); without it the
Spanish cities show the error and Austria still works.

## Data

- Spain: AEMET OpenData per municipality (INE code): the daily forecast
  (~7 days) and the hourly one (~3 days). Requests go through one queue
  paced for AEMET's rate limit: the capitals' days at start, a region's
  cities when it's opened, a city's hours when it's tapped, newest first.
- Austria: GeoSphere Austria Dataset API, `nwp-v2-1h-1km`, no key; ~60 hourly
  steps, shown as hours and folded into days. GeoSphere's weather symbol codes aren't published,
  so the sky comes from cloud cover, precipitation, snowfall and CAPE.
- Regions: Eurostat GISCO NUTS 2 (2021, 1:3M), © EuroGeographics, kept to ES
  and AT in `assets/*.geojson`. The Canary Islands are moved next to the
  peninsula.
- Icons: [Lucide](https://lucide.dev) 1.82.0 (ISC), converted with
  `cargo run -p svg2path -- <svg dir> meteo/assets/icons/`.

Map units are km, not dfui-map's metres: its tiles are sized for cities.
