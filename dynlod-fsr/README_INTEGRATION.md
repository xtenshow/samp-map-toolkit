# DynLOD + GTA SA FSR — optimized multicore analysis build

This package combines GTA SA FSR 1, the DynamicLOD adaptive frame-rate controller,
and an experimental native worker pipeline for scene-distance analysis.

## Main features

- GTA San Andreas Classic 1.0 US Compact and Hoodlum support (32-bit D3D9).
- FSR 1 EASU upscaling with optional RCAS sharpening and FXAA.
- Native-resolution post-effects and HUD after the low-resolution world pass.
- DynamicLOD adaptive FSR resolution and world-distance control.
- In-game /dyn menu, F12 fallback, F11 DynamicLOD toggle, and F10 FSR toggle.
- Live HUD with FPS, target FPS, CPU/RAM/GPU telemetry, output resolution,
  actual world render resolution, FSR scale, and scene-worker status.

## Multicore scene-distance analysis

The worker pipeline captures data from CRenderer::SetupMapEntityVisibility
(GTA SA 1.0 US address 0x553F60). GTA's original function executes first and
its result is preserved. The hook then copies only the computed distance
and visibility result into a thread-local per-frame buffer.

At the Present boundary, the frame's scalar samples are submitted to a
persistent pool of up to seven native worker threads. Workers classify
distances into near, mid, and far bands. Submission is non-blocking: if a
worker is still busy, its chunk is dropped instead of making the game thread wait.

### Safety and scope

- Worker threads receive only copied floating-point distances and chunk
  boundaries. They never dereference GTA entity/model pointers.
- Worker threads do not call RenderWare, Direct3D, physics, or game APIs.
- GTA rendering, simulation, and SA-MP networking remain on their original threads.
- The worker classifications currently provide parallel scene analysis and
  telemetry; they are not yet fed back into GTA's render-list decisions.
- This is not a fully multithreaded renderer and does not guarantee an FPS
  increase. The in-game counters verify that captured scene data is distributed.

The capture hot path batches counters at the frame boundary instead of doing
multiple global atomic updates or thread-ID system calls for every entity.
HUD worker statistics are refreshed at most four times per second to reduce
per-frame formatting overhead.

## Controls

- /dyn: open the settings menu.
- F12: fallback menu hotkey.
- F11: toggle DynamicLOD.
- F10: toggle FSR.
- /dyn hud, /dyn hud on, /dyn hud off: HUD control.

The DynamicLOD target is configured by dynamic_lod_target_fps in
gta_sa_fsr.ini. It is a performance target, not an FPS cap. The controller
adapts the internal render scale around that target.

## Install

1. Use GTA San Andreas Classic 1.0 US Compact or Hoodlum with an ASI loader.
2. Back up your current gta_sa_fsr.asi.
3. Copy the new gta_sa_fsr.asi into the GTA SA game folder.
4. Keep your existing gta_sa_fsr.ini so your personal settings, including
   your 300 FPS DynamicLOD target, are preserved. Do not overwrite it with
   the sample INI from the ZIP.
5. Start GTA SA / SA-MP and check gta_sa_fsr.log if anything does not load.

## Build and validation

GitHub Actions builds the 32-bit i686-pc-windows-msvc release, runs the Rust
unit tests, packages the ASI, and uploads the installable files. A successful
CI build confirms compilation and packaging; runtime stability and any FPS
benefit still need to be verified in the target GTA SA/SA-MP installation.
