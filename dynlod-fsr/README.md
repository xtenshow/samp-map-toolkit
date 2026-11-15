# DynLOD + GTA SA FSR (integrated ASI)

This folder contains GTA SA FSR 1 with the DynamicLOD adaptive FPS controller integrated into the same plugin. DynamicLOD is toggled with F11 by default and adjusts FSR custom render scale plus GTA far-clip/LOD distance.

The upstream FSR source and shader notices are retained. See upstream documentation below for supported executables and compatibility notes.

# GTA SA FSR

An experimental 32-bit ASI plugin that adds an FSR 1 upscaling path to classic
Grand Theft Auto: San Andreas. It renders the 3D world and depth-dependent
effects to low-resolution color and depth targets, upscales them with EASU, and
keeps the game's post-effects and HUD at native resolution.

## Current scope

- Classic 32-bit Direct3D 9 game only. The Definitive Edition is not supported.
- `Direct3DCreate9` bootstrap; no probe device.
- GTA SA 1.0 US Compact and Hoodlum executable signatures are supported.
- Narrow `CreateDevice` detour; the private `IDirect3D9` layout is not modified.
- Narrow function detours for `EndScene` and `Reset`; COM vtables stay untouched.
- The low-resolution frame ends before `CPostEffects::Render`, preventing native
  post-effect geometry from using the low-resolution viewport.
- Existing D3D9 hook chains are preserved for MoonLoader and other plugins.
- Optional fullscreen-borderless conversion with monitor-aware window sizing.
- Scalar Shader Model 3 ports of FSR 1 EASU and RCAS.
- Device-loss resource recreation.
- Auxiliary RenderWare targets are rejected by backbuffer identity checks.
- Non-full viewports are skipped to prevent cropping.
- INI configuration and diagnostic log.

The current `world_hud_easu` path is intended for visual and compatibility
validation. ReShade 6.8.0, MoonLoader, and Mod Loader have been validated in the
test environment. Other graphics wrappers and post-processing ASI plugins are
not guaranteed to be compatible.

## Build

Requirements:

- Rust toolchain with `i686-pc-windows-msvc` installed.
- Visual Studio Build Tools with the MSVC C++ toolchain and Windows SDK.
- [cargo-make](https://github.com/sagiegurari/cargo-make).

Run:

```text
cargo make
```

`cargo make` runs the default `package-asi` task. The package is written to `dist`.
The plugin links to the Windows D3D9 and D3DCompiler runtime components.

## Install

1. Install an ASI loader for the classic GTA SA executable.
2. Copy `gta_sa_fsr.asi` and `gta_sa_fsr.ini` from `dist` to the game directory.
3. Start the game and check `gta_sa_fsr.log` if the effect does not load.

## Configuration

Open `Options > Display Setup > Advanced > FSR Settings` to control the effect,
quality preset, custom render resolution, RCAS, and FXAA at runtime. Left,
Right, mouse wheel, and Enter cycle boolean values. Left/Right and the mouse
wheel cycle numeric and preset values; Enter advances them. Holding Shift
changes the Custom resolution step from 1% to 5%. Resolution displays the
effective percentage for every preset. It uses a 10% to 100% scale and is
editable only when the `Custom` quality preset is active. A bar visualizes the
effective scale, supports click-and-drag in Custom, and appears disabled outside
Custom. Drag commits the selected resolution and rebuilds render targets on
release. Values persist to
`gta_sa_fsr.ini`. Menu text lookup is isolated to stock menu
draw callsites so Mod Loader FXT entries remain untouched. The configured
`toggle_key` remains a non-persistent session toggle.

`quality` selects the source resolution used by EASU:

| Preset | Scale |
| --- | ---: |
| Ultra Quality | 0.77 |
| Quality | 0.67 |
| Balanced | 0.59 |
| Performance | 0.50 |

Set `quality=custom` to use `render_scale` directly. Existing configurations
without `quality` remain in custom mode.

`sharpness` uses FSR RCAS stops. `0.0` is the sharpest value and `2.0` is the
softest value. `rcas_enabled` and `fxaa_enabled` independently control the
native-resolution RCAS and FXAA passes.

`toggle_key` accepts `F1` through `F24`, letters, digits, arrows, and common
navigation keys such as `Insert`, `Home`, `End`, `PageUp`, `PageDown`, and
`Delete`. `auto_enable=false` waits for that key after the world becomes stable
and again after a device reset or game restart. Configuration is loaded once
when the plugin starts.

Set `benchmark_enabled=true` to enable the optional Present-based frame-time
benchmark. Press `benchmark_key` in a stable world. The benchmark discards
`benchmark_warmup_seconds`, samples for `benchmark_duration_seconds`, then
writes one result to `gta_sa_fsr.log` with average FPS, mean frame time,
p50/p95/p99 frame times, and 1% low FPS. Device resets, transitions, menus, and
FSR toggles abort the current run. The benchmark hook is not installed when the
option is disabled.

`pipeline_stage` selects `world_probe`, `world_resources`, `world_bind`,
`world_easu`, `world_hud_easu`, `single`, `compile`, `source`, `resources`,
`stretch`, `scene`, `easu`, or `full`.
`world_probe` instruments GTA's main and auxiliary `RenderScene` calls without
allocating resources or modifying render targets.
`world_resources` waits for `toggle_key` in an active world, then allocates paired
low-resolution color and depth surfaces without binding them.
`world_bind` redirects the world pass to those surfaces and restores the native
target before HUD rendering, but intentionally does not upscale the world.
`world_easu` upscales that low-resolution world into the native backbuffer before
the game's later effects and HUD passes. Internal RenderWare camera-update calls
reapply the low-resolution targets so split world passes stay in the FSR source.
`world_hud_easu` keeps later depth-dependent effects on the low-resolution
targets. It defers `CPostEffects::Render`, performs EASU, RCAS, and FXAA before
`Render2dStuff`, then renders the post-effects and HUD on the native target. It
also replays GTA's sky gradient and scales corona projection metadata while late
effects render in the low-resolution frame. EndScene and `CHud::Draw` provide
fallbacks. With `auto_enable=true`, the plugin activates automatically after
the world is stable. `toggle_key` then toggles the world upscaling path without
recreating its resources.
The staged modes isolate compatibility problems without changing hooks.
The diagnostic `source` stage waits for `toggle_key` before allocating its texture.
The low-memory `single` stage waits for a new-game transition, game-object
reinitialization, and a stable loaded world/player state,
then runs EASU directly into the backbuffer without an output texture or RCAS
pass. `toggle_key` toggles the active effect for visual comparison.

`borderless_fullscreen=true` converts GTA SA's exclusive-fullscreen device to a
borderless window on the nearest monitor. The setting is reapplied on D3D9
device resets and preserves the configured presentation interval.

## Known limitations

- Performance gains depend on the scene and system bottleneck.
- Some D3D9 wrappers may expose different device method addresses.
- ReShade presets must provide their own required textures, such as
  `UIMask.png` for `UIMask.fx`.
- FSR 2 and frame generation are out of scope because the game does not expose
  motion vectors or a suitable temporal rendering pipeline.
