# README media

User-facing stills live in `readme/` and are committed so GitHub can render
the README. Working takes (the demo video, ad-hoc stills) stay gitignored in
this directory.

```sh
bash scripts/capture-readme.sh
```

Isolated daemons, no login plugin, no desktop config. Needs a working desktop
OpenGL session. Stills are 640×480 (above the window's compact breakpoint).
Writes:

| File | What it is |
| --- | --- |
| `readme/window.png` | Live window, Glyphs |
| `readme/popover.png` | Bar popover, Glyphs |
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
