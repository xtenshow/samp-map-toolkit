# DynLOD + FSR integration

- One 32-bit ASI output: `gta_sa_fsr.asi`.
- FSR 1 world upscaling remains controlled through GTA's Advanced Display FSR menu.
- DynamicLOD is disabled by default and toggles with **F11** or the SA-MP command `/dyn on` / `/dyn off`.
- Use `/dyn` to open the rebuilt in-game menu. It has separate Controller, FSR/Render, World Distance, and HUD/Stats pages, each with a short Finglish description. Use mouse clicks, arrow keys, Enter, and Esc.
- Use `/dyn hud`, `/dyn hud on`, or `/dyn hud off` to control the performance HUD.
- The HUD shows current FPS, rolling average FPS, process CPU usage, process RAM working set, process GPU-engine utilization, actual render-target dimensions, output dimensions, and FSR scale. GPU reports N/A when Windows does not expose the process GPU counter. The 32-bit Windows build runs unit tests before packaging.
- Set `dynamic_lod_target_fps` in `gta_sa_fsr.ini` to any target (for example 75, 90, 120, or 144). This is a target, not an FPS cap. The controller adapts FSR render scale around it, and detects edits to this value live without restarting GTA.
- The controller samples the final Present frame boundary, applies hysteresis and asymmetric response/recovery, then changes FSR's custom render scale gradually.
- Render-target rebuilds are rate-limited to at least 3.5 seconds apart and require a larger scale delta to avoid frequent resolution changes.
- Far-clip and world LOD distances use the same adaptive quality multiplier. Lua-only SA-MP callbacks for nametags, 3D text, and chat bubbles are not part of this ASI build.
- Diagnostics are written to `gta_sa_fsr.log`.

## Build

The GitHub Actions workflow builds for `i686-pc-windows-msvc`, runs unit tests,
packages the plugin as an ASI, and publishes the installable files as a workflow artifact.

## Install

Use GTA San Andreas Classic 1.0 US Compact or Hoodlum with an ASI loader. Copy
`gta_sa_fsr.asi` and `gta_sa_fsr.ini` from the artifact into the same folder as
`gta_sa.exe`. A successful build is not a substitute for a runtime test on the target game installation.


Pull requests targeting `dynlod-fsr-build` are validated by the same 32-bit Windows build and unit-test workflow. The PR validation runs the same unit tests and release packaging steps as the branch build.
