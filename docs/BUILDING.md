# Building the toolkit

## Windows

Use Python 3.10 or newer. From the repository root:

```powershell
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install -r requirements-windows.txt
python src/windows/Radar_Tool_v8_DPI_Responsive.py
```

For a standalone EXE, open **Actions → Build Windows EXE** and run the workflow manually, or push a source change. Download the `RadarTool-Windows-EXE` artifact from a successful workflow run. Artifacts expire after 30 days.

## Android

The Kivy/KivyMD project is under `src/android/`. On GitHub, open **Actions → Build Android APK** and run the workflow, or push a change under `src/android/`. Download the `RadarTool-Android-APK` artifact from the successful run.

For a local build on a supported Linux environment:

```bash
cd src/android
python -m pip install buildozer==1.5.0 Cython==0.29.36
buildozer android debug
```

The Buildozer spec and requirements can be sensitive to Python, Android SDK/NDK, and dependency versions. Treat debug APKs as test builds.

## Before editing a map

1. Back up the original tile folder.
2. Verify that the set is numbered `radar00.png` through `radar143.png`.
3. Confirm tile dimensions before merging or splitting.
4. Test the output in a local copy of the game/resource before replacing a live server's map.

## Platform notes

- Windows is the desktop-oriented version.
- Android is an experimental port; verify document-provider and output-export behavior on the target phone.
- CI success confirms the build pipeline succeeded, not that all UI workflows have been exercised on a physical device.
