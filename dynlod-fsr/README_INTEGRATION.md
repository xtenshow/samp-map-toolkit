# DynLOD + FSR integration

- One 32-bit ASI output: `gta_sa_fsr.asi`.
- FSR 1 world upscaling remains controlled through GTA's Advanced Display FSR menu.
- DynamicLOD is disabled by default and toggles with **F11**.
- Set `dynamic_lod_target_fps` in `gta_sa_fsr.ini` to any target (for example 75, 90, 120, or 144). This is a target, not an FPS cap. The controller adapts FSR render scale around it, and detects edits to this value live without restarting GTA.
- The adaptive controller samples FPS, applies hysteresis and asymmetric response/recovery,
  then changes FSR's custom render scale gradually.
- Far-clip and LOD distances use the same adaptive quality multiplier.
- FSR render resources are not rebuilt more often than once every 1.5 seconds.
- Diagnostics are written to `gta_sa_fsr.log`.

## Build

The GitHub Actions workflow builds for `i686-pc-windows-msvc`, runs unit tests,
packages the plugin as an ASI, and publishes the installable files as a workflow artifact.

## Install

Use GTA San Andreas Classic 1.0 US Compact or Hoodlum with an ASI loader. Copy
`gta_sa_fsr.asi` and `gta_sa_fsr.ini` from the artifact into the same folder as
`gta_sa.exe`. The package has not been runtime-tested on the user's game installation.
