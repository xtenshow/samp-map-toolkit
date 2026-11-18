# DynLOD + FSR integration

- One 32-bit ASI output: `gta_sa_fsr.asi`.
- FSR 1 world upscaling remains controlled through GTA's Advanced Display FSR menu.
- DynamicLOD is disabled by default and toggles with **F11** or the SA-MP command `/dyn on` / `/dyn off`.
- Use `/dyn` to open the rebuilt in-game menu. **F12 is also a fallback menu hotkey** if a particular SA-MP 0.3.7 revision prevents command registration. It has separate Controller, FSR/Render, World Distance, and HUD/Stats pages. Use mouse clicks, arrow keys, Enter, and Esc.
- Use `/dyn hud`, `/dyn hud on`, or `/dyn hud off` to control the performance HUD.
- The HUD shows current FPS, rolling average FPS, process CPU usage, process RAM working set, process GPU-engine utilization, actual render-target dimensions, output dimensions, and FSR scale. GPU reports N/A when Windows does not expose the process GPU counter. The 32-bit Windows build runs unit tests before packaging.
- Set `dynamic_lod_target_fps` in `gta_sa_fsr.ini` to any target (for example 75, 90, 120, or 144). This is a target, not an FPS cap. The controller adapts FSR render scale around it, and detects edits to this value live without restarting GTA.
- The controller samples the final Present frame boundary, applies hysteresis and asymmetric response/recovery, then changes FSR's custom render scale gradually.
- Render-target rebuilds are rate-limited to at least 3.5 seconds apart and require a larger scale delta to avoid frequent resolution changes.
- Far-clip and world LOD distances use the same adaptive quality multiplier. Lua-only SA-MP callbacks for nametags, 3D text, and chat bubbles are not part of this ASI build.
- Diagnostics are written to `gta_sa_fsr.log`.

## Experimental multi-core scene preparation

- The ASI hooks RenderWare's `CVisibilityPlugins::RenderEntity` callback for the supported GTA SA 1.0 US executable and copies only its entity-distance scalar.
- A persistent worker pool (up to 7 threads, normally leaving one logical processor for GTA/SA-MP) splits the captured per-frame entity distances into chunks and computes distance bands and LOD-importance estimates concurrently.
- Worker threads never dereference GTA entity pointers, mutate game state, call RenderWare, or issue Direct3D commands. If a worker is busy, its chunk is dropped instead of blocking the game thread.
- The HUD reports active workers, the last logical processor observed by each worker, visible-entity samples, completed worker samples, and dropped chunks.
- This is a safe first multi-core experiment that moves a real per-entity scene-preparation analysis off the main thread. It does **not** make the original RenderWare draw calls, GTA simulation, or SA-MP networking multithreaded; expect no guaranteed FPS gain. The counters let us verify whether the analysis is being distributed before attempting deeper renderer changes.

## Build

The GitHub Actions workflow builds for `i686-pc-windows-msvc`, runs unit tests,
packages the plugin as an ASI, and publishes the installable files as a workflow artifact.

## Install

Use GTA San Andreas Classic 1.0 US Compact or Hoodlum with an ASI loader. Copy
`gta_sa_fsr.asi` and `gta_sa_fsr.ini` from the artifact into the same folder as
`gta_sa.exe`. A successful build is not a substitute for a runtime test on the target game installation.


Pull requests targeting `dynlod-fsr-build` are validated by the same 32-bit Windows build and unit-test workflow. The PR validation runs the same unit tests and release packaging steps as the branch build.
