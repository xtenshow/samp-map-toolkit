use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    pub enabled: bool,
    pub borderless_fullscreen: bool,
    pub pipeline_stage: PipelineStage,
    pub quality: QualityPreset,
    pub render_scale: f32,
    pub sharpness: f32,
    pub rcas_enabled: bool,
    pub fxaa_enabled: bool,
    pub toggle_key: u16,
    pub auto_enable: bool,
    pub benchmark_enabled: bool,
    pub benchmark_key: u16,
    pub benchmark_warmup_seconds: u32,
    pub benchmark_duration_seconds: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum QualityPreset {
    UltraQuality,
    Quality,
    Balanced,
    Performance,
    #[default]
    Custom,
}

impl QualityPreset {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UltraQuality => "ultra_quality",
            Self::Quality => "quality",
            Self::Balanced => "balanced",
            Self::Performance => "performance",
            Self::Custom => "custom",
        }
    }

    const fn from_runtime(value: u8) -> Self {
        match value {
            0 => Self::UltraQuality,
            1 => Self::Quality,
            2 => Self::Balanced,
            3 => Self::Performance,
            _ => Self::Custom,
        }
    }

    pub const fn cycle(self, direction: i8) -> Self {
        if direction < 0 {
            match self {
                Self::Performance => Self::Custom,
                Self::Balanced => Self::Performance,
                Self::Quality => Self::Balanced,
                Self::UltraQuality => Self::Quality,
                Self::Custom => Self::UltraQuality,
            }
        } else {
            match self {
                Self::Performance => Self::Balanced,
                Self::Balanced => Self::Quality,
                Self::Quality => Self::UltraQuality,
                Self::UltraQuality => Self::Custom,
                Self::Custom => Self::Performance,
            }
        }
    }
}

impl Config {
    pub fn effective_render_scale(self) -> f32 {
        match self.quality {
            QualityPreset::UltraQuality => 0.77,
            QualityPreset::Quality => 0.67,
            QualityPreset::Balanced => 0.59,
            QualityPreset::Performance => 0.50,
            QualityPreset::Custom => self.render_scale.clamp(0.1, 1.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum PipelineStage {
    WorldProbe,
    WorldResources,
    WorldBind,
    WorldEasu,
    WorldHudEasu,
    Compile,
    Source,
    Single,
    Resources,
    Stretch,
    Scene,
    Easu,
    #[default]
    Full,
}

impl PipelineStage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WorldProbe => "world_probe",
            Self::WorldResources => "world_resources",
            Self::WorldBind => "world_bind",
            Self::WorldEasu => "world_easu",
            Self::WorldHudEasu => "world_hud_easu",
            Self::Compile => "compile",
            Self::Source => "source",
            Self::Single => "single",
            Self::Resources => "resources",
            Self::Stretch => "stretch",
            Self::Scene => "scene",
            Self::Easu => "easu",
            Self::Full => "full",
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            borderless_fullscreen: false,
            pipeline_stage: PipelineStage::Full,
            quality: QualityPreset::Custom,
            render_scale: 0.75,
            sharpness: 0.2,
            rcas_enabled: true,
            fxaa_enabled: true,
            toggle_key: 0x79,
            auto_enable: true,
            benchmark_enabled: false,
            benchmark_key: 0x7A,
            benchmark_warmup_seconds: 5,
            benchmark_duration_seconds: 30,
        }
    }
}

static CONFIG: OnceLock<Config> = OnceLock::new();
static BASE_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();
static RUNTIME_ENABLED: AtomicBool = AtomicBool::new(true);
static RUNTIME_QUALITY: AtomicU8 = AtomicU8::new(QualityPreset::Custom as u8);
static RUNTIME_RENDER_SCALE: AtomicU32 = AtomicU32::new(0.75_f32.to_bits());
static RUNTIME_RCAS_ENABLED: AtomicBool = AtomicBool::new(true);
static RUNTIME_FXAA_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn initialize() {
    let _ = base_directory();
    let config = load();
    RUNTIME_ENABLED.store(config.enabled, Ordering::Release);
    RUNTIME_QUALITY.store(config.quality as u8, Ordering::Release);
    RUNTIME_RENDER_SCALE.store(config.render_scale.to_bits(), Ordering::Release);
    RUNTIME_RCAS_ENABLED.store(config.rcas_enabled, Ordering::Release);
    RUNTIME_FXAA_ENABLED.store(config.fxaa_enabled, Ordering::Release);
    let _ = CONFIG.set(config);
    log(&format!(
        "Loaded v{}: enabled={}, borderless_fullscreen={}, pipeline_stage={}, quality={}, render_scale={:.3}, sharpness={:.3}, rcas_enabled={}, fxaa_enabled={}, toggle_key=0x{:02X}, auto_enable={}, benchmark_enabled={}, benchmark_key=0x{:02X}, benchmark_warmup_seconds={}, benchmark_duration_seconds={}",
        env!("CARGO_PKG_VERSION"),
        config.enabled,
        config.borderless_fullscreen,
        config.pipeline_stage.as_str(),
        config.quality.as_str(),
        config.effective_render_scale(),
        config.sharpness,
        config.rcas_enabled,
        config.fxaa_enabled,
        config.toggle_key,
        config.auto_enable,
        config.benchmark_enabled,
        config.benchmark_key,
        config.benchmark_warmup_seconds,
        config.benchmark_duration_seconds
    ));
}

pub fn get() -> Config {
    let mut config = *CONFIG.get_or_init(load);
    config.enabled = RUNTIME_ENABLED.load(Ordering::Acquire);
    config.quality = QualityPreset::from_runtime(RUNTIME_QUALITY.load(Ordering::Acquire));
    config.render_scale = f32::from_bits(RUNTIME_RENDER_SCALE.load(Ordering::Acquire));
    config.rcas_enabled = RUNTIME_RCAS_ENABLED.load(Ordering::Acquire);
    config.fxaa_enabled = RUNTIME_FXAA_ENABLED.load(Ordering::Acquire);
    config
}

pub fn set_enabled(enabled: bool, persist: bool) -> Result<(), String> {
    RUNTIME_ENABLED.store(enabled, Ordering::Release);
    if persist {
        persist_setting("enabled", &enabled.to_string())?;
    }
    Ok(())
}

pub fn set_quality(quality: QualityPreset, persist: bool) -> Result<(), String> {
    RUNTIME_QUALITY.store(quality as u8, Ordering::Release);
    if persist {
        persist_setting("quality", quality.as_str())?;
    }
    Ok(())
}

pub fn set_render_scale(render_scale: f32, persist: bool) -> Result<(), String> {
    let render_scale = render_scale.clamp(0.1, 1.0);
    RUNTIME_RENDER_SCALE.store(render_scale.to_bits(), Ordering::Release);
    if persist {
        persist_setting("render_scale", &format!("{render_scale:.2}"))?;
    }
    Ok(())
}

pub fn set_rcas_enabled(enabled: bool, persist: bool) -> Result<(), String> {
    RUNTIME_RCAS_ENABLED.store(enabled, Ordering::Release);
    if persist {
        persist_setting("rcas_enabled", &enabled.to_string())?;
    }
    Ok(())
}

pub fn set_fxaa_enabled(enabled: bool, persist: bool) -> Result<(), String> {
    RUNTIME_FXAA_ENABLED.store(enabled, Ordering::Release);
    if persist {
        persist_setting("fxaa_enabled", &enabled.to_string())?;
    }
    Ok(())
}

pub fn log(message: &str) {
    let path = base_directory().join("gta_sa_fsr.log");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "[gta_sa_fsr] {message}");
    }
}

fn load() -> Config {
    let path = base_directory().join("gta_sa_fsr.ini");
    let Ok(contents) = fs::read_to_string(path) else {
        return Config::default();
    };

    parse(&contents)
}

pub fn persist_setting(key: &str, value: &str) -> Result<(), String> {
    let path = base_directory().join("gta_sa_fsr.ini");
    let contents = fs::read_to_string(&path).unwrap_or_default();
    fs::write(&path, update_setting(&contents, key, value))
        .map_err(|error| format!("write {}: {error}", path.display()))
}

fn update_setting(contents: &str, key: &str, value: &str) -> String {
    let newline = if contents.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut found = false;
    let mut lines = Vec::new();

    for line in contents.lines() {
        let comment_index = [line.find(';'), line.find('#')].into_iter().flatten().min();
        let (setting, comment) = comment_index
            .map(|index| (&line[..index], Some(&line[index..])))
            .unwrap_or((line, None));
        let is_target = setting
            .trim()
            .split_once('=')
            .is_some_and(|(source_key, _)| source_key.trim().eq_ignore_ascii_case(key));
        if is_target {
            let indent_length = line.len() - line.trim_start().len();
            let indent = &line[..indent_length];
            let suffix = comment
                .map(|comment| format!(" {comment}"))
                .unwrap_or_default();
            lines.push(format!("{indent}{key} = {value}{suffix}"));
            found = true;
        } else {
            lines.push(line.to_owned());
        }
    }

    if !found {
        lines.push(format!("{key} = {value}"));
    }

    let mut updated = lines.join(newline);
    updated.push_str(newline);
    updated
}

fn parse(contents: &str) -> Config {
    let mut config = Config::default();

    for source_line in contents.lines() {
        let line = source_line
            .split([';', '#'])
            .next()
            .unwrap_or_default()
            .trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        match key.trim().to_ascii_lowercase().as_str() {
            "enabled" => {
                if let Some(value) = parse_bool(value) {
                    config.enabled = value;
                }
            }
            "borderless_fullscreen" => {
                if let Some(value) = parse_bool(value) {
                    config.borderless_fullscreen = value;
                }
            }
            "pipeline_stage" => {
                config.pipeline_stage = match value.trim().to_ascii_lowercase().as_str() {
                    "world_probe" => PipelineStage::WorldProbe,
                    "world_resources" => PipelineStage::WorldResources,
                    "world_bind" => PipelineStage::WorldBind,
                    "world_easu" => PipelineStage::WorldEasu,
                    "world_hud_easu" => PipelineStage::WorldHudEasu,
                    "compile" => PipelineStage::Compile,
                    "source" => PipelineStage::Source,
                    "single" => PipelineStage::Single,
                    "resources" => PipelineStage::Resources,
                    "stretch" => PipelineStage::Stretch,
                    "scene" => PipelineStage::Scene,
                    "easu" => PipelineStage::Easu,
                    "full" => PipelineStage::Full,
                    _ => config.pipeline_stage,
                };
            }
            "quality" => {
                config.quality = match value.trim().to_ascii_lowercase().as_str() {
                    "ultra_quality" | "ultra quality" => QualityPreset::UltraQuality,
                    "quality" => QualityPreset::Quality,
                    "balanced" => QualityPreset::Balanced,
                    "performance" => QualityPreset::Performance,
                    "custom" => QualityPreset::Custom,
                    _ => config.quality,
                };
            }
            "render_scale" => {
                if let Ok(value) = value.trim().parse::<f32>()
                    && value.is_finite()
                {
                    config.render_scale = value.clamp(0.1, 1.0);
                }
            }
            "sharpness" => {
                if let Ok(value) = value.trim().parse::<f32>()
                    && value.is_finite()
                {
                    config.sharpness = value.clamp(0.0, 2.0);
                }
            }
            "rcas_enabled" => {
                if let Some(value) = parse_bool(value) {
                    config.rcas_enabled = value;
                }
            }
            "fxaa_enabled" => {
                if let Some(value) = parse_bool(value) {
                    config.fxaa_enabled = value;
                }
            }
            "toggle_key" => {
                if let Some(value) = parse_toggle_key(value) {
                    config.toggle_key = value;
                }
            }
            "auto_enable" => {
                if let Some(value) = parse_bool(value) {
                    config.auto_enable = value;
                }
            }
            "benchmark_enabled" => {
                if let Some(value) = parse_bool(value) {
                    config.benchmark_enabled = value;
                }
            }
            "benchmark_key" => {
                if let Some(value) = parse_toggle_key(value) {
                    config.benchmark_key = value;
                }
            }
            "benchmark_warmup_seconds" => {
                if let Ok(value) = value.trim().parse::<u32>() {
                    config.benchmark_warmup_seconds = value.clamp(0, 60);
                }
            }
            "benchmark_duration_seconds" => {
                if let Ok(value) = value.trim().parse::<u32>() {
                    config.benchmark_duration_seconds = value.clamp(5, 300);
                }
            }
            _ => {}
        }
    }

    config
}

fn parse_toggle_key(value: &str) -> Option<u16> {
    let key = value.trim().to_ascii_uppercase();
    if let Some(number) = key
        .strip_prefix('F')
        .and_then(|value| value.parse::<u16>().ok())
        && (1..=24).contains(&number)
    {
        return Some(0x70 + number - 1);
    }
    if key.len() == 1 {
        let byte = key.as_bytes()[0];
        if byte.is_ascii_uppercase() || byte.is_ascii_digit() {
            return Some(byte as u16);
        }
    }
    match key.as_str() {
        "INSERT" => Some(0x2D),
        "HOME" => Some(0x24),
        "END" => Some(0x23),
        "PAGEUP" | "PAGE_UP" => Some(0x21),
        "PAGEDOWN" | "PAGE_DOWN" => Some(0x22),
        "DELETE" => Some(0x2E),
        "SPACE" => Some(0x20),
        "TAB" => Some(0x09),
        "ENTER" => Some(0x0D),
        "ESC" | "ESCAPE" => Some(0x1B),
        "LEFT" => Some(0x25),
        "UP" => Some(0x26),
        "RIGHT" => Some(0x27),
        "DOWN" => Some(0x28),
        _ => None,
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

pub fn base_directory() -> &'static PathBuf {
    BASE_DIRECTORY.get_or_init(|| {
        std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(ToOwned::to_owned))
            .unwrap_or_else(|| PathBuf::from("."))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_clamps_values() {
        let config = parse(
            "\
             enabled = off\n\
             pipeline_stage = stretch\n\
             quality = performance\n\
             render_scale = 0.25\n\
             sharpness = 3.0\n\
             rcas_enabled = no\n\
             fxaa_enabled = off\n\
             toggle_key = F12\n\
             auto_enable = false\n\
             benchmark_enabled = true\n\
             benchmark_key = F6\n\
             benchmark_warmup_seconds = 99\n\
             benchmark_duration_seconds = 1\n",
        );

        assert_eq!(
            config,
            Config {
                enabled: false,
                borderless_fullscreen: false,
                pipeline_stage: PipelineStage::Stretch,
                quality: QualityPreset::Performance,
                render_scale: 0.25,
                sharpness: 2.0,
                rcas_enabled: false,
                fxaa_enabled: false,
                toggle_key: 0x7B,
                auto_enable: false,
                benchmark_enabled: true,
                benchmark_key: 0x75,
                benchmark_warmup_seconds: 60,
                benchmark_duration_seconds: 5,
            }
        );
        assert_eq!(config.effective_render_scale(), 0.5);
    }

    #[test]
    fn ignores_invalid_values() {
        let config = parse(
            "enabled=maybe\nquality=cinematic\nrender_scale=nan\nsharpness=nope\ntoggle_key=nope",
        );
        assert_eq!(config, Config::default());
    }

    #[test]
    fn maps_quality_presets_and_custom_scale() {
        for (name, expected) in [
            ("ultra_quality", 0.77),
            ("quality", 0.67),
            ("balanced", 0.59),
            ("performance", 0.50),
        ] {
            let config = parse(&format!("quality={name}\nrender_scale=0.91"));
            assert_eq!(config.effective_render_scale(), expected);
        }

        let config = parse("quality=custom\nrender_scale=0.83");
        assert_eq!(config.effective_render_scale(), 0.83);
    }

    #[test]
    fn parses_named_toggle_keys() {
        assert_eq!(parse_toggle_key("F1"), Some(0x70));
        assert_eq!(parse_toggle_key("F24"), Some(0x87));
        assert_eq!(parse_toggle_key("PageDown"), Some(0x22));
        assert_eq!(parse_toggle_key("K"), Some(b'K' as u16));
        assert_eq!(parse_toggle_key("7"), Some(b'7' as u16));
        assert_eq!(parse_toggle_key("F25"), None);
    }

    #[test]
    fn cycles_quality_presets_in_both_directions() {
        assert_eq!(QualityPreset::Performance.cycle(1), QualityPreset::Balanced);
        assert_eq!(QualityPreset::Performance.cycle(-1), QualityPreset::Custom);
        assert_eq!(QualityPreset::UltraQuality.cycle(1), QualityPreset::Custom);
        assert_eq!(QualityPreset::Custom.cycle(1), QualityPreset::Performance);
        assert_eq!(QualityPreset::Custom.cycle(-1), QualityPreset::UltraQuality);
    }

    #[test]
    fn updates_or_appends_settings_and_preserves_comments() {
        assert_eq!(
            update_setting(
                "; Header\r\nenabled = true ; master\r\nquality = performance\r\n",
                "enabled",
                "false"
            ),
            "; Header\r\nenabled = false ; master\r\nquality = performance\r\n"
        );
        assert_eq!(
            update_setting("quality = performance\n", "fxaa_enabled", "false"),
            "quality = performance\nfxaa_enabled = false\n"
        );
    }
}
