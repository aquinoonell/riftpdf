# riftpdf

A lightweight, keyboard-first PDF viewer for macOS.

No GTK, no traffic lights, continuous scroll, vim-style keys. Built for tiling window managers.

[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL%203.0-blue.svg)](LICENSE)

## Comparison

| | Preview | Zathura | **riftpdf** |
|--|:--:|:--:|:--:|
| Startup (same 433-page book)* | ~530 ms | ~270 ms | **~140 ms** |
| RAM settled* | ~186 MB | ~145 MB | **~125 MB** |
| Hide traffic lights | no | no | **yes** |
| Continuous scroll | yes | no | **yes** |
| Vim keys | no | yes | **yes** |

\*Measured on Apple Silicon opening *Zero to Production in Rust*; “ready” ≈ time until process RSS indicates the document is loaded.

## Requirements

- macOS
- [Rust](https://rustup.rs/) (`cargo`)

The first build compiles MuPDF and may take several minutes.

## Install

```bash
cargo install --git https://github.com/aquinoonell/riftpdf
```

From a local checkout:

```bash
cargo install --path .
```

```bash
riftpdf path/to/file.pdf
```

By default the process detaches from the terminal. Use `--foreground` / `-f` to keep it attached.

## Features

- Continuous vertical scroll
- Sticky zoom and pan (zoom toward center)
- Fit-width on open, then locks zoom so tiler layout switches keep magnification (`s` / `a` re-fit)
- Search with on-screen highlights (`/` · `n` / `N`)
- Table of contents (`t`)
- Clickable links (internal jumps; `https` / `mailto` via macOS `open`)
- Link highlights (`f` toggles all; hover always highlights)
- Remembers last page and manual zoom per file (reopens zoomed pages centered)
- Borderless window (drag from anywhere); native shadow disabled so JankyBorders remain visible

## Keys

| Key | Action |
|-----|--------|
| `j` `k` | Scroll down / up |
| `h` `l` | Pan left / right |
| `Ctrl-d` / `Ctrl-u` | Half-page down / up |
| `Ctrl-f` / `Ctrl-b` | Page down / up |
| `Ctrl-e` / `Ctrl-y` | Line down / up |
| `J` `K` / Space / PgUp / PgDn | Jump page |
| `gg` / `G` | First / last page |
| `+` `-` | Zoom in / out (centered) |
| `a` / `s` | Best-fit / fit-width (once, then lock) |
| `/` | Search |
| `n` / `N` | Next / previous match |
| `b` | Toggle status bar |
| `f` | Toggle link highlights |
| click | Follow link under cursor |
| `t` / `o` | Table of contents |
| `q` | Quit |
| Esc | Clear search · close TOC · quit |

### TOC mode

`j` / `k` move · Enter jump · type to filter · Esc close

## Development

```bash
cargo test
cargo build --release
./target/release/riftpdf --foreground file.pdf
```

CI on `master` / `main` runs `fmt`, `clippy -D warnings`, `test`, and `cargo deny`.

Pushing a `v*` tag publishes macOS release binaries (Apple Silicon and Intel) via GitHub Actions.

## License

AGPL-3.0 — required by the [MuPDF](https://mupdf.com/) bindings (`mupdf` crate).

## Status

macOS-focused. Search highlights are on-screen only (not written into the PDF). Not yet: annotations, Synctex, Windows/Linux packaging, Finder “Open with” `.app`.
