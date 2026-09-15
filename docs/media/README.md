# README media

User-facing stills live in `readme/` and are committed so GitHub can render
the README. Working takes (the demo video, ad-hoc stills) stay gitignored in
this directory.

```sh
bash scripts/capture-readme.sh
```

Isolated daemons, no login plugin, no desktop config. Needs a working desktop
OpenGL session. Windows are 640×480.

These are **presentation stills**, not a live take: they paint the archived
Moore/KTLX volume (`data/raw/KTLX20130520_201643_V06.gz`) with live chrome
(LIVE, a short age, no ARCHIVED badge) so the README shows a real storm
instead of whatever the feed is doing today. Tokyo Night is the dark theme;
Flexoki Light is the light one.

| File | What it is |
| --- | --- |
| `readme/hero.png` | Window + popover, Tokyo Night |
| `readme/window.png` | Window, Tokyo Night |
| `readme/window-light.png` | Window, Flexoki Light |
| `readme/themes.png` | Dark and light side by side |
| `readme/popover.png` | Bar popover, Tokyo Night |
| `readme/onboard.png` | First-run location prompt |
| `readme/search-city.png` | Search, city query |
| `readme/search-site.png` | Search, site id |
| `readme/search-coords.png` | Search, pasted coordinates |
| `readme/search-error.png` | Search, latitude out of range |
| `readme/locate-fail.png` | Approximate-location overlay |
| `readme/treatments.png` | Pixels, Glyphs, Stipple side by side |

`bash scripts/capture-demo.sh` still writes `omastorm-demo.mp4` and
`omastorm-preview.gif` here for omastorm.com; those are not in the README
until the take is recut against the current keys.

Radar: NOAA NEXRAD. Map: © OpenStreetMap contributors
([ODbL](https://opendatacommons.org/licenses/odbl/1-0/)); Natural Earth, public
domain.
