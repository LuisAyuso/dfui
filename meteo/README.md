# meteo

Daily and hourly forecasts for Spain and Austria on a map of their regions,
built with dfui and dfui-map. Phone-shaped: the map takes the top third, the
forecast table the rest.

```sh
cargo run -p meteo --release
cargo apk run -p meteo --lib --release   # Android, see docs/android.md
```

It opens on Tirol. The country shows its regional capitals; tap one (on the map or in the
table) and the map zooms to its region and shows the region's other cities.
Tap a city for its days, a day for its hours, and the back arrow to go up. Spain needs a free AEMET OpenData key
(<https://opendata.aemet.es/centrodedescargas/altaUsuario>), built in from
`meteo/aemet.key` (git-ignored) or given as `AEMET_API_KEY` at run time;
without it the Spanish cities show the error and Austria still works.

## Data

- Spain: AEMET OpenData's `todos` batches, every municipality's daily
  (~7 days, ~2 MB) and hourly (~2 days, ~6 MB) forecast as a .tar.gz; the
  cities are picked out by INE code. Madrid is fetched on its own first, as
  the batches take a while: eight requests in all. On a 429 it shows a wait
  bar and retries each minute; a dropped download is retried a few times.
- Austria: GeoSphere Austria Dataset API, `nwp-v2-1h-1km`, no key; ~60 hourly
  steps, shown as hours and folded into days. GeoSphere's weather symbol codes aren't published,
  so the sky comes from cloud cover, precipitation, snowfall and CAPE.
- Regions: Eurostat GISCO NUTS 2 (2021, 1:3M), © EuroGeographics, kept to ES
  and AT in `assets/*.geojson`. The Canary Islands are moved next to the
  peninsula.
- Icons: [Lucide](https://lucide.dev) 1.82.0 (ISC), converted with
  `cargo run -p svg2path -- <svg dir> meteo/assets/icons/`.

Map units are km, not dfui-map's metres: its tiles are sized for cities.
