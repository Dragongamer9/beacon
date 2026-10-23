# Third-party notices

Beacon is licensed under the GNU General Public License v3.0 or later (see LICENSE). Parts of it follow the design
of the projects below. No code from them is bundled or copied verbatim; their feature sets, file locations and data
formats were studied and reimplemented in Rust. They're credited here as their licenses ask.

## Faugus Launcher

- https://github.com/Faugus/faugus-launcher
- Copyright (c) Faugus
- License: MIT

Beacon's "your own games" feature (`crates/beacon-core/src/custom.rs`) rebuilds Faugus Launcher's feature set: adding
a Windows program or installer with its icon, a Wine prefix per game, the runner per game through umu-launcher,
GAMEID/STORE for protonfixes, MangoHud/GameMode/gamescope, Proton options, Wine tools, play time, and importing a
Faugus Launcher library (`games.json` and its `icons`, `covers` and `banners` folders, whose field names and layout
come from Faugus Launcher).

```
MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## ProtonPlus

- https://github.com/Vysp3r/ProtonPlus
- Copyright (c) Vysp3r and contributors
- License: GNU General Public License v3.0 (compatible with Beacon's GPL-3.0-or-later)

Beacon's Compatibility tools (`crates/beacon-core/src/tools.rs`) follow ProtonPlus's feature set: which tools to offer
(GE-Proton, Proton-CachyOS, Proton-EM, Proton-Sarek, UMU-Proton, Wine-GE, Kron4ek's Wine, DXVK, DXVK-async,
VKD3D-Proton), the folders Steam, Lutris, Heroic and Bottles (native and Flatpak) read them from, and a "Latest" copy
that updates itself. The full GPL-3.0 text is in Beacon's LICENSE file.

## icoextract

- https://github.com/jlu5/icoextract (MIT)

Faugus Launcher uses icoextract for program icons. Beacon has its own PE resource reader instead
(`crates/beacon-core/src/pe_icon.rs`), written from Microsoft's PE format documentation; it is credited here because it
does the same job.
