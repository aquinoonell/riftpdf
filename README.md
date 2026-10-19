# riftpdf

A lightweight, keyboard-first PDF viewer for macOS.

Built for speed and a clean tiling-WM workflow: no GTK, no traffic lights, continuous scroll, vim keys.

[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL%203.0-blue.svg)](LICENSE)

## Why

| | Preview | Zathura | **riftpdf** |
|--|:--:|:--:|:--:|
| Startup (same 433-page book)* | ~530 ms | ~270 ms | **~140 ms** |
| RAM settled* | ~186 MB | ~145 MB | **~125 MB** |
| Hide traffic lights | no | no | **yes** |
| Continuous scroll | yes | no | **yes** |
| Vim keys | no | yes | **yes** |

\*Measured on Apple Silicon opening *Zero to Production in Rust*; “ready” ≈ time until process RSS indicates the document is loaded.

## Install

**Needs:** macOS + [Rust](https://rustup.rs/) (`cargo`). First build compiles MuPDF and may take a few minutes.

```bash
cargo install --git https://github.com/aquinoonell/riftpdf
riftpdf path/to/file.pdf
```

Or from a clone:

```bash
git clone https://github.com/aquinoonell/riftpdf
cd riftpdf
cargo install --path .
```

Update later with `--force` on either install command.

## Features

- Continuous vertical scroll (pages flow as one document)
- Sticky zoom & pan (zoom toward center)
- Fit-width on open; re-fits when the window is tiled/resized
- Search with highlights (`/` · `n`/`N`)
- Table of contents (`t`)
- **Clickable links** (internal jumps + `https`/`mailto` via macOS `open`)
- Link highlights (`f` toggles all; hover always highlights)
- Remembers last page per file
- Borderless window (drag from anywhere)

## Keys

| Key | Action |
|-----|--------|
| `j` `k` | Scroll down / up |
| `h` `l` | Pan left / right |
| `J` `K` / Space / PgUp / PgDn | Jump page |
| `gg` / `G` | First / last page |
| `+` `-` | Zoom in / out (centered) |
| `a` / `s` | Best-fit / fit-width |
| `/` | Search |
| `n` / `N` | Next / previous match |
| `f` | Toggle link highlights |
| click | Follow link under cursor |
| `t` / `o` | Table of contents |
| `q` | Quit |
| Esc | Clear search · close TOC · quit |

### TOC mode
`j`/`k` move · Enter jump · type to filter · Esc close

## Demo

```bash
riftpdf ~/Documents/paper.pdf
```

## Build from source

```bash
cargo build --release
./target/release/riftpdf file.pdf
```

## License

AGPL-3.0 — required by the [MuPDF](https://mupdf.com/) bindings (`mupdf` crate).

## CI & releases

Every push / PR to `master` runs:

- `cargo fmt --check`
- `cargo clippy -D warnings`
- `cargo test`
- `cargo deny` (licenses + advisories)

Tag a version to cut a GitHub Release with macOS binaries (Apple Silicon + Intel):

```bash
git tag v0.1.0
git push origin v0.1.0
```

## Status

macOS-focused. Not yet: Synctex, annotations, Windows/Linux packaging, Finder “Open with” `.app`.
