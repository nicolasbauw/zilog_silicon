# zilog_silicon

Shared SDL2/wgpu/egui frontend framework, extracted from [ByteBox](https://github.com/nicolasbauw/amstrad_cpc)'s
own frontend and now also used by [trust-80](https://github.com/nicolasbauw/TRS-80). It exists because both
emulators needed the same plumbing around their very different CPU cores: a
window, a wgpu render pipeline with an optional CRT shader, and a handful of
secondary egui-on-wgpu panels (a status window, a log console). Rather than
maintain two copies of that plumbing, it lives here once.

## What this crate provides

The library target (`zilog_silicon::*`) is deliberately narrow - just the
parts of a frontend that don't know or care what machine is being emulated:

- **`renderer`** - a wgpu render pipeline that letterboxes an RGB24
  framebuffer of arbitrary size into whatever the actual window size is, with
  an optional CRT shader (scanlines + aperture mask) on top. `CrtSettings` is
  a plain data struct (mask/scanline/blur parameters); this crate has no
  opinion on where those values come from or where they're saved.
- **`egui_gpu`** - the wiring to run egui on its own wgpu surface inside a
  *secondary* SDL2 window (a status panel, a debug console) - each such
  window owns its own device/swapchain, independent of the main window's.
- **`console_log`** - a tiny global log queue (`app_log!`-style), so code
  deep in a machine core can log something without threading a console
  reference through every function signature.
- **`status_panel`** - a generic scrollable egui window that renders two
  preformatted text blocks (e.g. "registers" and "hardware status") with
  monospace styling - the caller decides what text goes in it.
- **`ui_scale`** - a content-scaling helper (font size, spacing) for egui
  panels, driven by the real window size, so a panel doesn't stay tiny in
  fullscreen on a large/high-DPI display.

The shared WGSL shaders (`renderer_frame.wgsl`, `renderer_crt.wgsl`) are
plain fullscreen-quad shaders with no consumer-specific code in them, so
they're portable verbatim even to a target this crate itself doesn't support
(a wasm/web frontend hooks into them through its own `egui_wgpu::CallbackTrait`
integration instead of this crate's `Renderer` - see trust-80-web).

This is a library-only crate: it originally also carried a `bytebox` bin
target (ByteBox's own frontend binary, assembled from CPC-specific
keyboard/config/console panels, the SDL2 event loop, audio...), kept here
temporarily while ByteBox itself still had its own pre-extraction copies of
the shared modules to compare against. Once ByteBox was migrated to depend
on this crate directly (like trust-80 already did), that bin - and
everything only it needed (bytebox-core, image, rfd, zilog_z80, the
packaging assets/build script) - moved back out, since ByteBox's own repo
is where a CPC-specific frontend binary belongs.

## What this crate does *not* provide

- No CPU emulation, no bus/memory/keyboard-matrix logic - that's each
  emulator's own core crate (`bytebox-core`, `trust-80-core`).
- No `config.toml` (or any other persistence) handling - `CrtSettings` and
  friends are plain structs; serializing them is the consumer's job (see
  e.g. `amstrad_cpc/bytebox/src/config_panel.rs`'s
  `crt_settings_from_config`/`crt_settings_to_config`).
- No window creation or event loop - the consumer owns `sdl2::init()`, its
  main window, and its own `event_pump`; this crate only reacts to what
  it's handed (a frame buffer to present, an `sdl2::event::Event` to relay
  to `egui_gpu`).
- No keyboard/gamepad-to-machine mapping - that's inherently
  machine-specific (a CPC keyboard matrix and a TRS-80 one don't look
  alike) and stays in each consumer.

## Status

Used in production by two independent frontends (ByteBox, trust-80) since
their own frontend rewrites - the shared surface (`renderer`, `egui_gpu`,
`console_log`, `status_panel`, `ui_scale`) has been exercised, and had bugs
found and fixed, through both. Versioned 1.0 on that basis, though with the
usual caveat that a two-consumer, single-maintainer, unpublished crate's
"stable API" is more a statement of intent than an enforced contract.

## Using this crate

Not published to crates.io - it's an internal shared library for this
author's own emulators, and publishing would add ceremony (semver
discipline enforced by the ecosystem, yanking policy...) for zero benefit
to a crate with two consumers under one roof. The GitHub repo itself is
public (there's nothing sensitive in it - no ROMs, no credentials, just
generic wgpu/egui/SDL2 plumbing), which keeps this simple: a plain HTTPS
git dependency, no auth of any kind needed anywhere, including CI.
Consumers depend on it via a git dependency tracking `branch = "master"`:

```toml
zilog_silicon = { git = "https://github.com/nicolasbauw/zilog_silicon.git", branch = "master" }
```
