use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU32, Ordering};

use windows::Win32::Foundation::{HANDLE, RECT};
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCOMPILE_OPTIMIZATION_LEVEL3, D3DCompile};
use windows::Win32::Graphics::Direct3D::{ID3DBlob, ID3DInclude};
use windows::Win32::Graphics::Direct3D9::*;
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::core::{Interface, PCSTR, s};

use crate::config::{self, PipelineStage};
use crate::game;
use crate::shaders;

thread_local! {
    static RENDERER: RefCell<Renderer> = const { RefCell::new(Renderer::new()) };
}

static NATIVE_CLEAR_COLOR: AtomicU32 = AtomicU32::new(0);

pub fn process_frame(device: &IDirect3DDevice9) -> Result<(), String> {
    RENDERER.with(|renderer| renderer.borrow_mut().process_frame(device))
}

pub fn invalidate() {
    RENDERER.with(|renderer| renderer.borrow_mut().invalidate());
}

pub fn probe_world_render(device: &IDirect3DDevice9) -> Result<(), String> {
    RENDERER.with(|renderer| renderer.borrow_mut().probe_world_render(device))
}

pub fn prepare_world_resources(device: &IDirect3DDevice9) -> Result<(), String> {
    RENDERER.with(|renderer| renderer.borrow_mut().prepare_world_resources(device))
}

pub fn begin_world_render_for_frame(
    device: &IDirect3DDevice9,
    frame_blocked: bool,
) -> Result<bool, String> {
    RENDERER.with(|renderer| {
        renderer
            .borrow_mut()
            .begin_world_render(device, frame_blocked)
    })
}

pub fn sample_outer_world_state(stable: bool) {
    RENDERER.with(|renderer| renderer.borrow_mut().sample_outer_world_state(stable));
}

pub fn end_world_render(device: &IDirect3DDevice9) -> Result<(), String> {
    RENDERER.with(|renderer| renderer.borrow_mut().end_world_render(device))
}

pub fn world_frame_active() -> bool {
    RENDERER.with(|renderer| renderer.borrow().world_frame_state.is_some())
}

pub fn world_effect_ready() -> bool {
    RENDERER.with(|renderer| {
        let renderer = renderer.borrow();
        config::get().enabled
            && renderer.world_resources.is_some()
            && renderer.world_effect_enabled
            && renderer.outer_world_stable
            && renderer.world_stable_frames >= 30
    })
}

pub fn effect_enabled_for_menu() -> bool {
    RENDERER.with(|renderer| {
        let renderer = renderer.borrow();
        config::get().enabled
            && (renderer.world_resources.is_none() || renderer.world_effect_enabled)
    })
}

pub fn set_effect_enabled_from_menu(enabled: bool) {
    RENDERER.with(|renderer| {
        let mut renderer = renderer.borrow_mut();
        if renderer.world_resources.is_some() {
            renderer.world_effect_enabled = enabled;
        }
        if enabled {
            renderer.disabled = false;
        }
    });
}

pub fn request_quality_rebuild() {
    RENDERER.with(|renderer| {
        renderer.borrow_mut().world_quality_rebuild_pending = true;
    });
}

pub fn world_render_dimensions() -> Option<(u32, u32)> {
    RENDERER.with(|renderer| {
        let renderer = renderer.borrow();
        renderer.world_frame_state.as_ref()?;
        renderer
            .world_resources
            .as_ref()
            .map(|resources| (resources.width, resources.height))
    })
}

/// Returns the low-resolution world targets only while they are actively used.
pub fn active_world_render_dimensions() -> Option<(u32, u32, u32, u32)> {
    RENDERER.with(|renderer| {
        let renderer = renderer.borrow();
        if renderer.disabled
            || !config::get().enabled
            || !renderer.world_effect_enabled
            || !renderer.outer_world_stable
            || renderer.world_stable_frames < 30
        {
            return None;
        }
        renderer.world_resources.as_ref().map(|resources| (
            resources.width, resources.height, resources.output_width, resources.output_height
        ))
    })
}

pub fn rebind_world_after_camera_update(device: &IDirect3DDevice9) -> Result<(), String> {
    RENDERER.with(|renderer| {
        renderer
            .borrow_mut()
            .rebind_world_after_camera_update(device)
    })
}

pub fn capture_native_clear_color(color: u32) {
    NATIVE_CLEAR_COLOR.store(color, Ordering::Release);
}

fn world_effect_enabled_message(settings: config::Config) -> &'static str {
    match (settings.rcas_enabled, settings.fxaa_enabled) {
        (true, true) => "World EASU+RCAS+FXAA enabled",
        (true, false) => "World EASU+RCAS enabled",
        (false, true) => "World EASU+FXAA enabled",
        (false, false) => "World EASU enabled",
    }
}

fn world_effect_enabled_automatically_message(settings: config::Config) -> &'static str {
    match (settings.rcas_enabled, settings.fxaa_enabled) {
        (true, true) => "World EASU+RCAS+FXAA enabled automatically",
        (true, false) => "World EASU+RCAS enabled automatically",
        (false, true) => "World EASU+FXAA enabled automatically",
        (false, false) => "World EASU enabled automatically",
    }
}

fn world_effect_completed_message(settings: config::Config) -> &'static str {
    match (settings.rcas_enabled, settings.fxaa_enabled) {
        (true, true) => {
            "World and late effects EASU+RCAS+FXAA completed before native post-effects and HUD"
        }
        (true, false) => {
            "World and late effects EASU+RCAS completed before native post-effects and HUD"
        }
        (false, true) => {
            "World and late effects EASU+FXAA completed before native post-effects and HUD"
        }
        (false, false) => {
            "World and late effects EASU completed before native post-effects and HUD"
        }
    }
}

struct Renderer {
    resources: Option<Resources>,
    single_pass_resources: Option<SinglePassResources>,
    diagnostic_source: Option<(IDirect3DTexture9, IDirect3DSurface9)>,
    disabled: bool,
    first_pass_logged: bool,
    manual_source_armed_logged: bool,
    activation_requested: bool,
    automatic_wait_logged: bool,
    ready_candidate_frames: u32,
    ready_candidate_ped: usize,
    single_pass_enabled: bool,
    world_effect_enabled: bool,
    outer_world_stable: bool,
    world_stable_frames: u32,
    world_transition_suspended: bool,
    toggle_key_was_down: bool,
    unsupported_viewport_logged: bool,
    world_probe_main_logged: bool,
    world_probe_aux_logged: bool,
    world_probe_other_logged: bool,
    world_resources: Option<WorldResources>,
    world_quality_rebuild_pending: bool,
    world_resources_armed_logged: bool,
    world_manual_ready_logged: bool,
    world_frame_state: Option<WorldFrameState>,
    world_redirect_logged: bool,
    world_camera_rebind_count: u32,
    world_clear_logged: bool,
}

impl Renderer {
    const fn new() -> Self {
        Self {
            resources: None,
            single_pass_resources: None,
            diagnostic_source: None,
            disabled: false,
            first_pass_logged: false,
            manual_source_armed_logged: false,
            activation_requested: false,
            automatic_wait_logged: false,
            ready_candidate_frames: 0,
            ready_candidate_ped: 0,
            single_pass_enabled: false,
            world_effect_enabled: false,
            outer_world_stable: false,
            world_stable_frames: 0,
            world_transition_suspended: false,
            toggle_key_was_down: false,
            unsupported_viewport_logged: false,
            world_probe_main_logged: false,
            world_probe_aux_logged: false,
            world_probe_other_logged: false,
            world_resources: None,
            world_quality_rebuild_pending: false,
            world_resources_armed_logged: false,
            world_manual_ready_logged: false,
            world_frame_state: None,
            world_redirect_logged: false,
            world_camera_rebind_count: 0,
            world_clear_logged: false,
        }
    }

    fn process_frame(&mut self, device: &IDirect3DDevice9) -> Result<(), String> {
        let settings = config::get();
        if self.disabled || !settings.enabled {
            return Ok(());
        }

        if matches!(
            settings.pipeline_stage,
            PipelineStage::WorldProbe
                | PipelineStage::WorldResources
                | PipelineStage::WorldBind
                | PipelineStage::WorldEasu
                | PipelineStage::WorldHudEasu
        ) {
            return Ok(());
        }

        if settings.pipeline_stage == PipelineStage::Single {
            let toggle_pressed = self.poll_toggle_pressed();
            if self.single_pass_resources.is_none() && !self.activation_requested {
                if !self.automatic_wait_logged {
                    config::log(if settings.auto_enable {
                        "Automatic activation waiting for stable world state"
                    } else {
                        "Manual activation waiting for stable world state and the toggle key"
                    });
                    self.automatic_wait_logged = true;
                }
                match game::ready_player_ped() {
                    Some(player_ped) if player_ped == self.ready_candidate_ped => {
                        self.ready_candidate_frames = self.ready_candidate_frames.saturating_add(1);
                    }
                    Some(player_ped) => {
                        self.ready_candidate_ped = player_ped;
                        self.ready_candidate_frames = 1;
                    }
                    None => {
                        self.ready_candidate_ped = 0;
                        self.ready_candidate_frames = 0;
                    }
                }
                if self.ready_candidate_frames >= 30 && (settings.auto_enable || toggle_pressed) {
                    config::log(if settings.auto_enable {
                        "Stable world state confirmed; automatic activation requested"
                    } else {
                        "Stable world state confirmed; manual activation requested"
                    });
                    self.activation_requested = true;
                } else {
                    if toggle_pressed && self.ready_candidate_frames < 30 {
                        config::log("Toggle key ignored until the world state is stable");
                    }
                    return Ok(());
                }
            } else if self.single_pass_resources.is_some() && toggle_pressed {
                self.single_pass_enabled = !self.single_pass_enabled;
                config::log(if self.single_pass_enabled {
                    "Single-pass EASU enabled"
                } else {
                    "Single-pass EASU disabled"
                });
            }
            if self.single_pass_resources.is_some() && !self.single_pass_enabled {
                return Ok(());
            }
        }

        let render_target = unsafe { device.GetRenderTarget(0) }
            .map_err(|error| format!("GetRenderTarget: {error}"))?;
        let back_buffer = unsafe { device.GetBackBuffer(0, 0, D3DBACKBUFFER_TYPE_MONO) }
            .map_err(|error| format!("GetBackBuffer: {error}"))?;

        // RenderWare uses BeginScene/EndScene for small auxiliary rasters too.
        // Only process the swap-chain backbuffer, never those temporary targets.
        if render_target.as_raw() != back_buffer.as_raw() {
            return Ok(());
        }

        let mut description = D3DSURFACE_DESC::default();
        unsafe { render_target.GetDesc(&mut description) }
            .map_err(|error| format!("Get render target description: {error}"))?;
        let mut viewport = D3DVIEWPORT9::default();
        unsafe { device.GetViewport(&mut viewport) }
            .map_err(|error| format!("GetViewport: {error}"))?;

        if viewport.X != 0
            || viewport.Y != 0
            || viewport.Width != description.Width
            || viewport.Height != description.Height
        {
            if !self.unsupported_viewport_logged {
                config::log(&format!(
                    "Skipped non-full viewport: {},{} {}x{} on {}x{} target",
                    viewport.X,
                    viewport.Y,
                    viewport.Width,
                    viewport.Height,
                    description.Width,
                    description.Height
                ));
                self.unsupported_viewport_logged = true;
            }
            return Ok(());
        }

        if settings.pipeline_stage == PipelineStage::Compile {
            compile_shader_bytecode(shaders::EASU, "FSR EASU")?;
            compile_shader_bytecode(shaders::RCAS, "FSR RCAS")?;
            compile_shader_bytecode(shaders::FXAA, "FXAA")?;
            config::log("Pipeline stage completed: compile; renderer suspended");
            self.disabled = true;
            return Ok(());
        }

        if settings.pipeline_stage == PipelineStage::Source {
            if !self.manual_source_armed_logged {
                config::log(
                    "Manual source allocation armed; press the toggle key after world load",
                );
                self.manual_source_armed_logged = true;
            }
            let toggle_pressed = self.poll_toggle_pressed();
            if !toggle_pressed {
                return Ok(());
            }
            config::log("Manual source allocation requested");
            let input_width = ((description.Width as f32 * settings.effective_render_scale())
                .round() as u32)
                .max(1);
            let input_height = ((description.Height as f32 * settings.effective_render_scale())
                .round() as u32)
                .max(1);
            let texture =
                create_render_texture(device, input_width, input_height, description.Format)?;
            let surface = unsafe { texture.GetSurfaceLevel(0) }
                .map_err(|error| format!("Get diagnostic source surface: {error}"))?;
            self.diagnostic_source = Some((texture, surface));
            config::log(&format!(
                "Pipeline stage completed: source {}x{}; renderer suspended",
                input_width, input_height
            ));
            self.disabled = true;
            return Ok(());
        }

        if settings.pipeline_stage == PipelineStage::Single {
            if self.single_pass_resources.is_none() {
                self.single_pass_resources = Some(SinglePassResources::create(
                    device,
                    description.Width,
                    description.Height,
                    description.Format,
                    settings.effective_render_scale(),
                )?);
                self.single_pass_enabled = true;
            }
            let result = self.apply_single_pass(device, &render_target, &viewport);
            if result.is_ok() && !self.first_pass_logged {
                config::log("Single-pass EASU completed");
                self.first_pass_logged = true;
            }
            if let Err(error) = &result {
                self.disabled = true;
                config::log(&format!(
                    "Single-pass EASU disabled until device reset: {error}"
                ));
            }
            return result;
        }

        self.ensure_resources(
            device,
            description.Width,
            description.Height,
            description.Format,
            settings.effective_render_scale(),
        )?;

        if settings.pipeline_stage == PipelineStage::Resources {
            if !self.first_pass_logged {
                config::log("Pipeline stage completed: resources; renderer suspended");
                self.first_pass_logged = true;
            }
            // One-shot diagnostic: retain the allocations but stop all later
            // render-target and viewport queries from the EndScene callback.
            self.disabled = true;
            return Ok(());
        }

        let result =
            self.apply_post_process(device, &render_target, &viewport, settings.pipeline_stage);
        if result.is_ok() && !self.first_pass_logged {
            config::log(&format!(
                "Pipeline stage completed: {}",
                settings.pipeline_stage.as_str()
            ));
            self.first_pass_logged = true;
        }
        if let Err(error) = &result {
            self.disabled = true;
            config::log(&format!(
                "Post-process disabled until device reset: {error}"
            ));
        }
        result
    }

    fn probe_world_render(&mut self, device: &IDirect3DDevice9) -> Result<(), String> {
        if !config::get().enabled || !game::world_render_probe_ready() {
            return Ok(());
        }

        let render_target = unsafe { device.GetRenderTarget(0) }
            .map_err(|error| format!("GetRenderTarget: {error}"))?;
        let back_buffer = unsafe { device.GetBackBuffer(0, 0, D3DBACKBUFFER_TYPE_MONO) }
            .map_err(|error| format!("GetBackBuffer: {error}"))?;
        let auxiliary = game::auxiliary_render_scene_active();
        let on_backbuffer = render_target.as_raw() == back_buffer.as_raw();
        let already_logged = if auxiliary {
            &mut self.world_probe_aux_logged
        } else if on_backbuffer {
            &mut self.world_probe_main_logged
        } else {
            &mut self.world_probe_other_logged
        };
        if *already_logged {
            return Ok(());
        }

        let mut target_description = D3DSURFACE_DESC::default();
        unsafe { render_target.GetDesc(&mut target_description) }
            .map_err(|error| format!("Get render target description: {error}"))?;
        let depth_description =
            unsafe { device.GetDepthStencilSurface() }
                .ok()
                .and_then(|surface| {
                    let mut description = D3DSURFACE_DESC::default();
                    unsafe { surface.GetDesc(&mut description) }
                        .ok()
                        .map(|()| description)
                });
        let mut viewport = D3DVIEWPORT9::default();
        unsafe { device.GetViewport(&mut viewport) }
            .map_err(|error| format!("GetViewport: {error}"))?;

        let depth = depth_description
            .map(|description| {
                format!(
                    "{}x{} {:?}",
                    description.Width, description.Height, description.Format
                )
            })
            .unwrap_or_else(|| "none".to_owned());
        config::log(&format!(
            "RenderScene probe: auxiliary={auxiliary}, backbuffer={on_backbuffer}, target={}x{} {:?}, depth={depth}, viewport={},{} {}x{}",
            target_description.Width,
            target_description.Height,
            target_description.Format,
            viewport.X,
            viewport.Y,
            viewport.Width,
            viewport.Height
        ));
        *already_logged = true;
        Ok(())
    }

    fn prepare_world_resources(&mut self, device: &IDirect3DDevice9) -> Result<(), String> {
        let settings = config::get();
        let toggle_pressed = self.poll_toggle_pressed();
        if self.disabled || !settings.enabled || game::auxiliary_render_scene_active() {
            return Ok(());
        }

        let world_stable = self.outer_world_stable;
        if self.world_quality_rebuild_pending && self.world_resources.is_none() {
            self.world_quality_rebuild_pending = false;
        }
        if self.world_quality_rebuild_pending
            && world_stable
            && self.world_frame_state.is_none()
            && self.world_resources.is_some()
        {
            self.world_resources = None;
            self.world_effect_enabled = false;
            self.ready_candidate_frames = 0;
            self.ready_candidate_ped = 0;
            self.world_stable_frames = 0;
            self.world_resources_armed_logged = false;
            self.world_manual_ready_logged = false;
            self.world_redirect_logged = false;
            self.world_quality_rebuild_pending = false;
            config::log("World FSR quality change accepted; resources will be rebuilt");
        }
        if self.world_resources.is_some() {
            if toggle_pressed {
                if !world_stable {
                    config::log("Toggle key ignored during transition or screen fade");
                    return Ok(());
                }
                if self.world_frame_state.is_some() {
                    self.end_world_render(device)?;
                    config::log("World frame finished before toggle");
                }
                self.world_effect_enabled = !self.world_effect_enabled;
                config::log(if self.world_effect_enabled {
                    world_effect_enabled_message(settings)
                } else {
                    "World FSR disabled"
                });
            }
            return Ok(());
        }

        if !world_stable {
            self.ready_candidate_ped = 0;
            self.ready_candidate_frames = 0;
            if toggle_pressed {
                config::log("Toggle key ignored until world activation is safe");
            }
            return Ok(());
        }

        let render_target = unsafe { device.GetRenderTarget(0) }
            .map_err(|error| format!("Get world render target: {error}"))?;
        let back_buffer = unsafe { device.GetBackBuffer(0, 0, D3DBACKBUFFER_TYPE_MONO) }
            .map_err(|error| format!("Get world backbuffer: {error}"))?;
        if render_target.as_raw() != back_buffer.as_raw() {
            return Ok(());
        }
        if !self.world_resources_armed_logged {
            config::log(if settings.auto_enable {
                "Automatic world FSR activation waiting for stable world state"
            } else {
                "Manual world FSR activation waiting for stable world state"
            });
            self.world_resources_armed_logged = true;
        }

        match game::world_activation_player_ped() {
            Some(player_ped) if player_ped == self.ready_candidate_ped => {
                self.ready_candidate_frames = self.ready_candidate_frames.saturating_add(1);
            }
            Some(player_ped) => {
                self.ready_candidate_ped = player_ped;
                self.ready_candidate_frames = 1;
            }
            None => {
                self.ready_candidate_ped = 0;
                self.ready_candidate_frames = 0;
            }
        }
        if self.ready_candidate_frames < 30 {
            if toggle_pressed {
                config::log("Toggle key ignored until world activation is safe");
            }
            return Ok(());
        }
        if !settings.auto_enable && !toggle_pressed {
            if !self.world_manual_ready_logged {
                config::log("Stable world state confirmed; press the toggle key to enable FSR");
                self.world_manual_ready_logged = true;
            }
            return Ok(());
        }
        config::log("Stable world state confirmed; creating FSR resources");

        let mut color_description = D3DSURFACE_DESC::default();
        unsafe { render_target.GetDesc(&mut color_description) }
            .map_err(|error| format!("Get world color description: {error}"))?;
        let native_depth = unsafe { device.GetDepthStencilSurface() }
            .map_err(|error| format!("Get world depth surface: {error}"))?;
        let mut depth_description = D3DSURFACE_DESC::default();
        unsafe { native_depth.GetDesc(&mut depth_description) }
            .map_err(|error| format!("Get world depth description: {error}"))?;
        let scale = settings.effective_render_scale();
        let input_width = ((color_description.Width as f32 * scale).round() as u32).max(1);
        let input_height = ((color_description.Height as f32 * scale).round() as u32).max(1);

        let color_texture =
            create_render_texture(device, input_width, input_height, color_description.Format)?;
        let color_surface = unsafe { color_texture.GetSurfaceLevel(0) }
            .map_err(|error| format!("Get world color surface: {error}"))?;
        let depth_surface =
            create_depth_surface(device, input_width, input_height, depth_description.Format)?;
        let easu_shader = compile_shader(device, shaders::EASU, "world FSR EASU")?;
        let easu_output_texture = create_render_texture(
            device,
            color_description.Width,
            color_description.Height,
            color_description.Format,
        )?;
        let easu_output_surface = unsafe { easu_output_texture.GetSurfaceLevel(0) }
            .map_err(|error| format!("Get world EASU output surface: {error}"))?;
        let rcas_shader = compile_shader(device, shaders::RCAS, "world FSR RCAS")?;
        let rcas_output_texture = create_render_texture(
            device,
            color_description.Width,
            color_description.Height,
            color_description.Format,
        )?;
        let rcas_output_surface = unsafe { rcas_output_texture.GetSurfaceLevel(0) }
            .map_err(|error| format!("Get world RCAS output surface: {error}"))?;
        let fxaa_shader = compile_shader(device, shaders::FXAA, "world FXAA")?;
        self.world_resources = Some(WorldResources {
            color_texture,
            color_surface,
            depth_surface,
            easu_shader,
            easu_output_texture,
            easu_output_surface,
            rcas_shader,
            rcas_output_texture,
            rcas_output_surface,
            fxaa_shader,
            width: input_width,
            height: input_height,
            output_width: color_description.Width,
            output_height: color_description.Height,
            clear_stencil: matches!(
                depth_description.Format,
                D3DFMT_D15S1 | D3DFMT_D24S8 | D3DFMT_D24FS8
            ),
        });
        self.world_effect_enabled = true;
        self.world_stable_frames = 30;
        self.world_transition_suspended = false;
        config::log(&format!(
            "World FSR resources created: {}x{} {:?} + {:?} -> {}x{}",
            input_width,
            input_height,
            color_description.Format,
            depth_description.Format,
            color_description.Width,
            color_description.Height
        ));
        config::log(if settings.auto_enable {
            world_effect_enabled_automatically_message(settings)
        } else {
            world_effect_enabled_message(settings)
        });
        Ok(())
    }

    fn begin_world_render(
        &mut self,
        device: &IDirect3DDevice9,
        frame_blocked: bool,
    ) -> Result<bool, String> {
        if frame_blocked {
            self.poll_toggle_pressed();
            return Ok(false);
        }
        self.prepare_world_resources(device)?;
        if self.disabled
            || !config::get().enabled
            || self.world_resources.is_none()
            || !self.world_effect_enabled
            || !self.outer_world_stable
            || self.world_stable_frames < 30
            || game::auxiliary_render_scene_active()
            || self.world_frame_state.is_some()
        {
            return Ok(false);
        }

        let native_color = unsafe { device.GetRenderTarget(0) }
            .map_err(|error| format!("Get native world target: {error}"))?;
        let back_buffer = unsafe { device.GetBackBuffer(0, 0, D3DBACKBUFFER_TYPE_MONO) }
            .map_err(|error| format!("Get native world backbuffer: {error}"))?;
        if native_color.as_raw() != back_buffer.as_raw() {
            return Ok(false);
        }
        let native_depth = unsafe { device.GetDepthStencilSurface() }
            .map_err(|error| format!("Get native world depth: {error}"))?;
        let mut native_viewport = D3DVIEWPORT9::default();
        unsafe { device.GetViewport(&mut native_viewport) }
            .map_err(|error| format!("Get native world viewport: {error}"))?;
        let mut native_description = D3DSURFACE_DESC::default();
        unsafe { native_color.GetDesc(&mut native_description) }
            .map_err(|error| format!("Get native world target description: {error}"))?;
        if native_viewport.X != 0
            || native_viewport.Y != 0
            || native_viewport.Width != native_description.Width
            || native_viewport.Height != native_description.Height
        {
            if !self.unsupported_viewport_logged {
                config::log(&format!(
                    "Skipped world FSR on non-full viewport: {},{} {}x{} on {}x{} target",
                    native_viewport.X,
                    native_viewport.Y,
                    native_viewport.Width,
                    native_viewport.Height,
                    native_description.Width,
                    native_description.Height
                ));
                self.unsupported_viewport_logged = true;
            }
            return Ok(false);
        }
        let mut native_scissor = RECT::default();
        unsafe { device.GetScissorRect(&mut native_scissor) }
            .map_err(|error| format!("Get native world scissor: {error}"))?;
        let mut native_scissor_enabled = 0;
        unsafe { device.GetRenderState(D3DRS_SCISSORTESTENABLE, &mut native_scissor_enabled) }
            .map_err(|error| format!("Get native scissor state: {error}"))?;

        let frame_state = WorldFrameState {
            native_color,
            native_depth,
            native_viewport,
            native_scissor,
            native_scissor_enabled,
        };
        let resources = self
            .world_resources
            .as_ref()
            .expect("world resources checked above");
        let low_viewport = D3DVIEWPORT9 {
            X: 0,
            Y: 0,
            Width: resources.width,
            Height: resources.height,
            MinZ: 0.0,
            MaxZ: 1.0,
        };
        let low_scissor = RECT {
            left: 0,
            top: 0,
            right: resources.width as i32,
            bottom: resources.height as i32,
        };
        let clear_flags = (D3DCLEAR_TARGET | D3DCLEAR_ZBUFFER) as u32
            | if resources.clear_stencil {
                D3DCLEAR_STENCIL as u32
            } else {
                0
            };
        let bind_result = (|| -> Result<(), String> {
            unsafe { device.SetDepthStencilSurface(None::<&IDirect3DSurface9>) }
                .map_err(|error| format!("Detach native world depth: {error}"))?;
            unsafe { device.SetRenderTarget(0, &resources.color_surface) }
                .map_err(|error| format!("Bind low-resolution world color: {error}"))?;
            unsafe { device.SetDepthStencilSurface(&resources.depth_surface) }
                .map_err(|error| format!("Bind low-resolution world depth: {error}"))?;
            unsafe { device.SetViewport(&low_viewport) }
                .map_err(|error| format!("Set low-resolution world viewport: {error}"))?;
            unsafe { device.SetScissorRect(&low_scissor) }
                .map_err(|error| format!("Set low-resolution world scissor: {error}"))?;
            unsafe { device.SetRenderState(D3DRS_SCISSORTESTENABLE, 0) }
                .map_err(|error| format!("Disable low-resolution world scissor: {error}"))?;
            let clear_color = NATIVE_CLEAR_COLOR.load(Ordering::Acquire);
            unsafe { device.Clear(0, null_mut(), clear_flags, clear_color, 1.0, 0) }
                .map_err(|error| format!("Clear low-resolution world targets: {error}"))
        })();
        if let Err(error) = bind_result {
            let restore_error = restore_world_frame(device, &frame_state).err();
            self.disabled = true;
            return Err(match restore_error {
                Some(restore) => format!("{error}; native restore also failed: {restore}"),
                None => error,
            });
        }

        self.world_frame_state = Some(frame_state);
        if !self.world_clear_logged {
            config::log(&format!(
                "Low-resolution world clear color: 0x{:08X}",
                NATIVE_CLEAR_COLOR.load(Ordering::Acquire)
            ));
            self.world_clear_logged = true;
        }
        Ok(true)
    }

    fn sample_outer_world_state(&mut self, stable: bool) {
        self.outer_world_stable = stable;
        if self.world_resources.is_none() {
            if !stable {
                self.ready_candidate_frames = 0;
                self.ready_candidate_ped = 0;
            }
            return;
        }

        if stable {
            self.world_stable_frames = self.world_stable_frames.saturating_add(1).min(30);
            if self.world_stable_frames >= 30 && self.world_transition_suspended {
                self.world_transition_suspended = false;
                config::log("World FSR resumed after stable transition recovery");
            }
        } else {
            self.world_stable_frames = 0;
            if !self.world_transition_suspended {
                self.world_transition_suspended = true;
                config::log("World FSR suspended during transition or screen fade");
            }
        }
    }

    fn end_world_render(&mut self, device: &IDirect3DDevice9) -> Result<(), String> {
        let Some(frame_state) = self.world_frame_state.take() else {
            return Ok(());
        };
        let settings = config::get();
        let world_easu = matches!(
            settings.pipeline_stage,
            PipelineStage::WorldEasu | PipelineStage::WorldHudEasu
        );
        let result = if world_easu {
            let resources = self
                .world_resources
                .as_ref()
                .ok_or_else(|| "world resources disappeared during RenderScene".to_owned())?;
            upscale_world_to_native(
                device,
                resources,
                &frame_state,
                settings.pipeline_stage == PipelineStage::WorldHudEasu && settings.rcas_enabled,
                settings.pipeline_stage == PipelineStage::WorldHudEasu && settings.fxaa_enabled,
            )
        } else {
            restore_world_frame(device, &frame_state)
        };
        if result.is_ok() && !self.world_redirect_logged {
            config::log(if settings.pipeline_stage == PipelineStage::WorldHudEasu {
                world_effect_completed_message(settings)
            } else if world_easu {
                "World EASU completed before native effects and HUD"
            } else {
                "World rendered to low-resolution targets; native target restored"
            });
            self.world_redirect_logged = true;
        }
        if result.is_err() {
            self.disabled = true;
        }
        result
    }

    fn rebind_world_after_camera_update(
        &mut self,
        device: &IDirect3DDevice9,
    ) -> Result<(), String> {
        if self.disabled || self.world_frame_state.is_none() {
            return Ok(());
        }
        let Some(resources) = self.world_resources.as_ref() else {
            return Ok(());
        };
        let color_surface = resources.color_surface.clone();
        let depth_surface = resources.depth_surface.clone();
        let width = resources.width;
        let height = resources.height;
        let current_target = unsafe { device.GetRenderTarget(0) }
            .map_err(|error| format!("Get post-camera-update target: {error}"))?;
        let was_low = current_target.as_raw() == color_surface.as_raw();
        let viewport = D3DVIEWPORT9 {
            X: 0,
            Y: 0,
            Width: width,
            Height: height,
            MinZ: 0.0,
            MaxZ: 1.0,
        };
        let scissor = RECT {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        let rebind_result = (|| -> Result<(), String> {
            unsafe { device.SetDepthStencilSurface(None::<&IDirect3DSurface9>) }
                .map_err(|error| format!("Detach camera-update depth: {error}"))?;
            unsafe { device.SetRenderTarget(0, &color_surface) }
                .map_err(|error| format!("Rebind low-resolution world color: {error}"))?;
            unsafe { device.SetDepthStencilSurface(&depth_surface) }
                .map_err(|error| format!("Rebind low-resolution world depth: {error}"))?;
            unsafe { device.SetViewport(&viewport) }
                .map_err(|error| format!("Rebind low-resolution world viewport: {error}"))?;
            unsafe { device.SetScissorRect(&scissor) }
                .map_err(|error| format!("Rebind low-resolution world scissor: {error}"))?;
            unsafe { device.SetRenderState(D3DRS_SCISSORTESTENABLE, 0) }
                .map_err(|error| format!("Disable rebound world scissor: {error}"))
        })();

        if let Err(error) = rebind_result {
            let restore_error = self
                .world_frame_state
                .take()
                .and_then(|state| restore_world_frame(device, &state).err());
            self.disabled = true;
            return Err(match restore_error {
                Some(restore) => format!("{error}; native restore also failed: {restore}"),
                None => error,
            });
        }

        if self.world_camera_rebind_count < 4 {
            config::log(&format!(
                "RwCameraBeginUpdate low-target rebind: target_was_low={was_low}, viewport={width}x{height}"
            ));
        }
        self.world_camera_rebind_count = self.world_camera_rebind_count.saturating_add(1);
        Ok(())
    }

    fn apply_post_process(
        &self,
        device: &IDirect3DDevice9,
        render_target: &IDirect3DSurface9,
        original_viewport: &D3DVIEWPORT9,
        pipeline_stage: PipelineStage,
    ) -> Result<(), String> {
        let resources = self.resources.as_ref().expect("resources initialized");
        let depth_stencil = unsafe { device.GetDepthStencilSurface() }.ok();
        let state = unsafe { device.CreateStateBlock(D3DSBT_ALL) }
            .map_err(|error| format!("Capture D3D9 state: {error}"))?;

        let stretch_result = unsafe {
            device.StretchRect(
                render_target,
                std::ptr::null(),
                &resources.source_surface,
                std::ptr::null(),
                D3DTEXF_LINEAR,
            )
        }
        .map_err(|error| format!("Downsample backbuffer: {error}"));

        let begin_result = if stretch_result.is_ok() && pipeline_stage >= PipelineStage::Scene {
            unsafe { device.BeginScene() }
                .map_err(|error| format!("Begin post-process scene: {error}"))
        } else {
            Ok(())
        };

        let draw_result = if stretch_result.is_ok()
            && begin_result.is_ok()
            && pipeline_stage >= PipelineStage::Easu
        {
            (|| -> Result<(), String> {
                unsafe { device.SetRenderTarget(0, &resources.easu_surface) }
                    .map_err(|error| format!("Set EASU target: {error}"))?;
                unsafe { device.SetDepthStencilSurface(None::<&IDirect3DSurface9>) }
                    .map_err(|error| format!("Disable depth for EASU: {error}"))?;
                set_output_viewport(device, resources.output_width, resources.output_height)?;
                set_fullscreen_state(device)?;

                let source_texture = resources
                    .source_texture
                    .cast::<IDirect3DBaseTexture9>()
                    .map_err(|error| format!("Cast EASU input texture: {error}"))?;
                unsafe { device.SetTexture(0, &source_texture) }
                    .map_err(|error| format!("Bind EASU input: {error}"))?;
                unsafe { device.SetPixelShader(&resources.easu_shader) }
                    .map_err(|error| format!("Bind EASU shader: {error}"))?;
                let easu_constants = [
                    resources.input_width as f32,
                    resources.input_height as f32,
                    1.0 / resources.input_width as f32,
                    1.0 / resources.input_height as f32,
                ];
                unsafe { device.SetPixelShaderConstantF(0, easu_constants.as_ptr(), 1) }
                    .map_err(|error| format!("Set EASU constants: {error}"))?;
                draw_fullscreen_quad(device, resources.output_width, resources.output_height)?;

                if pipeline_stage == PipelineStage::Full {
                    unsafe { device.SetRenderTarget(0, render_target) }
                        .map_err(|error| format!("Restore output target for RCAS: {error}"))?;
                    let easu_texture = resources
                        .easu_texture
                        .cast::<IDirect3DBaseTexture9>()
                        .map_err(|error| format!("Cast RCAS input texture: {error}"))?;
                    unsafe { device.SetTexture(0, &easu_texture) }
                        .map_err(|error| format!("Bind RCAS input: {error}"))?;
                    unsafe { device.SetPixelShader(&resources.rcas_shader) }
                        .map_err(|error| format!("Bind RCAS shader: {error}"))?;
                    let sharpness_coefficient = 2.0_f32.powf(-config::get().sharpness);
                    let rcas_constants = [
                        1.0 / resources.output_width as f32,
                        1.0 / resources.output_height as f32,
                        sharpness_coefficient,
                        0.0,
                    ];
                    unsafe { device.SetPixelShaderConstantF(0, rcas_constants.as_ptr(), 1) }
                        .map_err(|error| format!("Set RCAS constants: {error}"))?;
                    draw_fullscreen_quad(device, resources.output_width, resources.output_height)?;
                }
                Ok(())
            })()
        } else {
            Ok(())
        };

        let end_result = if begin_result.is_ok()
            && stretch_result.is_ok()
            && pipeline_stage >= PipelineStage::Scene
        {
            unsafe { device.EndScene() }.map_err(|error| format!("End post-process scene: {error}"))
        } else {
            Ok(())
        };

        unsafe { device.SetTexture(0, None::<&IDirect3DBaseTexture9>) }.ok();
        let state_result =
            unsafe { state.Apply() }.map_err(|error| format!("Restore D3D9 state: {error}"));
        let target_result = unsafe { device.SetRenderTarget(0, render_target) }
            .map_err(|error| format!("Restore render target: {error}"));
        let depth_result = match &depth_stencil {
            Some(depth) => unsafe { device.SetDepthStencilSurface(depth) },
            None => unsafe { device.SetDepthStencilSurface(None::<&IDirect3DSurface9>) },
        }
        .map_err(|error| format!("Restore depth surface: {error}"));
        let viewport_result = unsafe { device.SetViewport(original_viewport) }
            .map_err(|error| format!("Restore viewport: {error}"));

        stretch_result?;
        begin_result?;
        draw_result?;
        end_result?;
        state_result?;
        target_result?;
        depth_result?;
        viewport_result
    }

    fn apply_single_pass(
        &self,
        device: &IDirect3DDevice9,
        render_target: &IDirect3DSurface9,
        original_viewport: &D3DVIEWPORT9,
    ) -> Result<(), String> {
        let resources = self
            .single_pass_resources
            .as_ref()
            .expect("single-pass resources initialized");
        let depth_stencil = unsafe { device.GetDepthStencilSurface() }.ok();
        let state = unsafe { device.CreateStateBlock(D3DSBT_ALL) }
            .map_err(|error| format!("Capture single-pass state: {error}"))?;

        let stretch_result = unsafe {
            device.StretchRect(
                render_target,
                std::ptr::null(),
                &resources.source_surface,
                std::ptr::null(),
                D3DTEXF_LINEAR,
            )
        }
        .map_err(|error| format!("Downsample single-pass source: {error}"));

        let begin_result = if stretch_result.is_ok() {
            unsafe { device.BeginScene() }
                .map_err(|error| format!("Begin single-pass scene: {error}"))
        } else {
            Ok(())
        };

        let draw_result = if stretch_result.is_ok() && begin_result.is_ok() {
            (|| -> Result<(), String> {
                unsafe { device.SetRenderTarget(0, render_target) }
                    .map_err(|error| format!("Bind single-pass output: {error}"))?;
                unsafe { device.SetDepthStencilSurface(None::<&IDirect3DSurface9>) }
                    .map_err(|error| format!("Disable single-pass depth: {error}"))?;
                set_output_viewport(device, resources.output_width, resources.output_height)?;
                set_fullscreen_state(device)?;

                let source_texture = resources
                    .source_texture
                    .cast::<IDirect3DBaseTexture9>()
                    .map_err(|error| format!("Cast single-pass input texture: {error}"))?;
                unsafe { device.SetTexture(0, &source_texture) }
                    .map_err(|error| format!("Bind single-pass input: {error}"))?;
                unsafe { device.SetPixelShader(&resources.easu_shader) }
                    .map_err(|error| format!("Bind single-pass EASU shader: {error}"))?;
                let constants = [
                    resources.input_width as f32,
                    resources.input_height as f32,
                    1.0 / resources.input_width as f32,
                    1.0 / resources.input_height as f32,
                ];
                unsafe { device.SetPixelShaderConstantF(0, constants.as_ptr(), 1) }
                    .map_err(|error| format!("Set single-pass EASU constants: {error}"))?;
                draw_fullscreen_quad(device, resources.output_width, resources.output_height)
            })()
        } else {
            Ok(())
        };

        let end_result = if stretch_result.is_ok() && begin_result.is_ok() {
            unsafe { device.EndScene() }.map_err(|error| format!("End single-pass scene: {error}"))
        } else {
            Ok(())
        };

        unsafe { device.SetTexture(0, None::<&IDirect3DBaseTexture9>) }.ok();
        let state_result =
            unsafe { state.Apply() }.map_err(|error| format!("Restore single-pass state: {error}"));
        let target_result = unsafe { device.SetRenderTarget(0, render_target) }
            .map_err(|error| format!("Restore single-pass render target: {error}"));
        let depth_result = match &depth_stencil {
            Some(depth) => unsafe { device.SetDepthStencilSurface(depth) },
            None => unsafe { device.SetDepthStencilSurface(None::<&IDirect3DSurface9>) },
        }
        .map_err(|error| format!("Restore single-pass depth surface: {error}"));
        let viewport_result = unsafe { device.SetViewport(original_viewport) }
            .map_err(|error| format!("Restore single-pass viewport: {error}"));

        stretch_result?;
        begin_result?;
        draw_result?;
        end_result?;
        state_result?;
        target_result?;
        depth_result?;
        viewport_result
    }

    fn ensure_resources(
        &mut self,
        device: &IDirect3DDevice9,
        output_width: u32,
        output_height: u32,
        color_format: D3DFORMAT,
        scale: f32,
    ) -> Result<(), String> {
        let input_width = ((output_width as f32 * scale).round() as u32).max(1);
        let input_height = ((output_height as f32 * scale).round() as u32).max(1);
        let reusable = self.resources.as_ref().is_some_and(|resources| {
            resources.input_width == input_width
                && resources.input_height == input_height
                && resources.output_width == output_width
                && resources.output_height == output_height
                && resources.color_format == color_format
        });
        if reusable {
            return Ok(());
        }

        self.resources = None;
        self.resources = Some(Resources::create(
            device,
            input_width,
            input_height,
            output_width,
            output_height,
            color_format,
        )?);
        config::log(&format!(
            "Post-process resources: {}x{} -> {}x{}",
            input_width, input_height, output_width, output_height
        ));
        Ok(())
    }

    fn invalidate(&mut self) {
        self.resources = None;
        self.single_pass_resources = None;
        self.diagnostic_source = None;
        self.disabled = false;
        self.first_pass_logged = false;
        self.manual_source_armed_logged = false;
        self.activation_requested = false;
        self.automatic_wait_logged = false;
        self.ready_candidate_frames = 0;
        self.ready_candidate_ped = 0;
        self.single_pass_enabled = false;
        self.world_effect_enabled = false;
        self.outer_world_stable = false;
        self.world_stable_frames = 0;
        self.world_transition_suspended = false;
        self.toggle_key_was_down = false;
        self.unsupported_viewport_logged = false;
        self.world_probe_main_logged = false;
        self.world_probe_aux_logged = false;
        self.world_probe_other_logged = false;
        self.world_resources = None;
        self.world_quality_rebuild_pending = false;
        self.world_resources_armed_logged = false;
        self.world_manual_ready_logged = false;
        self.world_frame_state = None;
        self.world_redirect_logged = false;
        self.world_camera_rebind_count = 0;
        self.world_clear_logged = false;
    }

    fn poll_toggle_pressed(&mut self) -> bool {
        let is_down = unsafe { GetAsyncKeyState(config::get().toggle_key as i32) } < 0;
        let pressed = is_down && !self.toggle_key_was_down;
        self.toggle_key_was_down = is_down;
        pressed
    }
}

struct Resources {
    input_width: u32,
    input_height: u32,
    output_width: u32,
    output_height: u32,
    color_format: D3DFORMAT,
    source_texture: IDirect3DTexture9,
    source_surface: IDirect3DSurface9,
    easu_texture: IDirect3DTexture9,
    easu_surface: IDirect3DSurface9,
    easu_shader: IDirect3DPixelShader9,
    rcas_shader: IDirect3DPixelShader9,
}

struct SinglePassResources {
    input_width: u32,
    input_height: u32,
    output_width: u32,
    output_height: u32,
    source_texture: IDirect3DTexture9,
    source_surface: IDirect3DSurface9,
    easu_shader: IDirect3DPixelShader9,
}

struct WorldResources {
    color_texture: IDirect3DTexture9,
    color_surface: IDirect3DSurface9,
    depth_surface: IDirect3DSurface9,
    easu_shader: IDirect3DPixelShader9,
    easu_output_texture: IDirect3DTexture9,
    easu_output_surface: IDirect3DSurface9,
    rcas_shader: IDirect3DPixelShader9,
    rcas_output_texture: IDirect3DTexture9,
    rcas_output_surface: IDirect3DSurface9,
    fxaa_shader: IDirect3DPixelShader9,
    width: u32,
    height: u32,
    output_width: u32,
    output_height: u32,
    clear_stencil: bool,
}

struct WorldFrameState {
    native_color: IDirect3DSurface9,
    native_depth: IDirect3DSurface9,
    native_viewport: D3DVIEWPORT9,
    native_scissor: RECT,
    native_scissor_enabled: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WorldPipelinePlan {
    easu_to_intermediate: bool,
    apply_rcas: bool,
    rcas_to_intermediate: bool,
    apply_fxaa: bool,
    fxaa_from_rcas: bool,
}

const fn world_pipeline_plan(apply_rcas: bool, apply_fxaa: bool) -> WorldPipelinePlan {
    WorldPipelinePlan {
        easu_to_intermediate: apply_rcas || apply_fxaa,
        apply_rcas,
        rcas_to_intermediate: apply_rcas && apply_fxaa,
        apply_fxaa,
        fxaa_from_rcas: apply_rcas && apply_fxaa,
    }
}

fn upscale_world_to_native(
    device: &IDirect3DDevice9,
    resources: &WorldResources,
    state: &WorldFrameState,
    apply_rcas: bool,
    apply_fxaa: bool,
) -> Result<(), String> {
    let plan = world_pipeline_plan(apply_rcas, apply_fxaa);
    let post_world_state = unsafe { device.CreateStateBlock(D3DSBT_ALL) }
        .map_err(|error| format!("Capture post-world D3D9 state: {error}"))?;
    let draw_result = (|| -> Result<(), String> {
        unsafe { device.SetDepthStencilSurface(None::<&IDirect3DSurface9>) }
            .map_err(|error| format!("Detach low-resolution depth for EASU: {error}"))?;
        let easu_target = if plan.easu_to_intermediate {
            &resources.easu_output_surface
        } else {
            &state.native_color
        };
        unsafe { device.SetRenderTarget(0, easu_target) }
            .map_err(|error| format!("Bind world EASU output: {error}"))?;
        unsafe { device.SetViewport(&state.native_viewport) }
            .map_err(|error| format!("Set native world EASU viewport: {error}"))?;
        set_fullscreen_state(device)?;

        let source_texture = resources
            .color_texture
            .cast::<IDirect3DBaseTexture9>()
            .map_err(|error| format!("Cast low-resolution world texture: {error}"))?;
        unsafe { device.SetTexture(0, &source_texture) }
            .map_err(|error| format!("Bind low-resolution world texture: {error}"))?;
        unsafe { device.SetPixelShader(&resources.easu_shader) }
            .map_err(|error| format!("Bind world EASU shader: {error}"))?;
        let constants = [
            resources.width as f32,
            resources.height as f32,
            1.0 / resources.width as f32,
            1.0 / resources.height as f32,
        ];
        unsafe { device.SetPixelShaderConstantF(0, constants.as_ptr(), 1) }
            .map_err(|error| format!("Set world EASU constants: {error}"))?;
        draw_fullscreen_quad(
            device,
            state.native_viewport.Width,
            state.native_viewport.Height,
        )?;

        if !plan.apply_rcas && !plan.apply_fxaa {
            return Ok(());
        }

        if plan.apply_rcas {
            unsafe { device.SetTexture(0, None::<&IDirect3DBaseTexture9>) }
                .map_err(|error| format!("Unbind world EASU input: {error}"))?;
            let rcas_target = if plan.rcas_to_intermediate {
                &resources.rcas_output_surface
            } else {
                &state.native_color
            };
            unsafe { device.SetRenderTarget(0, rcas_target) }
                .map_err(|error| format!("Bind world RCAS output: {error}"))?;
            unsafe { device.SetViewport(&state.native_viewport) }
                .map_err(|error| format!("Set native world RCAS viewport: {error}"))?;
            set_fullscreen_state(device)?;

            let easu_output_texture = resources
                .easu_output_texture
                .cast::<IDirect3DBaseTexture9>()
                .map_err(|error| format!("Cast world EASU output texture: {error}"))?;
            unsafe { device.SetTexture(0, &easu_output_texture) }
                .map_err(|error| format!("Bind world EASU output for RCAS: {error}"))?;
            unsafe { device.SetPixelShader(&resources.rcas_shader) }
                .map_err(|error| format!("Bind world RCAS shader: {error}"))?;
            let rcas_constants = [
                1.0 / resources.output_width as f32,
                1.0 / resources.output_height as f32,
                2.0_f32.powf(-config::get().sharpness),
                0.0,
            ];
            unsafe { device.SetPixelShaderConstantF(0, rcas_constants.as_ptr(), 1) }
                .map_err(|error| format!("Set world RCAS constants: {error}"))?;
            draw_fullscreen_quad(device, resources.output_width, resources.output_height)?;
        }

        if !plan.apply_fxaa {
            return Ok(());
        }

        unsafe { device.SetTexture(0, None::<&IDirect3DBaseTexture9>) }
            .map_err(|error| format!("Unbind world pre-FXAA input: {error}"))?;
        unsafe { device.SetRenderTarget(0, &state.native_color) }
            .map_err(|error| format!("Bind native world output for FXAA: {error}"))?;
        unsafe { device.SetViewport(&state.native_viewport) }
            .map_err(|error| format!("Set native world FXAA viewport: {error}"))?;
        set_fullscreen_state(device)?;

        let fxaa_input_texture = if plan.fxaa_from_rcas {
            resources
                .rcas_output_texture
                .cast::<IDirect3DBaseTexture9>()
                .map_err(|error| format!("Cast world RCAS output texture: {error}"))?
        } else {
            resources
                .easu_output_texture
                .cast::<IDirect3DBaseTexture9>()
                .map_err(|error| format!("Cast world EASU output texture for FXAA: {error}"))?
        };
        unsafe { device.SetTexture(0, &fxaa_input_texture) }
            .map_err(|error| format!("Bind world input for FXAA: {error}"))?;
        unsafe { device.SetPixelShader(&resources.fxaa_shader) }
            .map_err(|error| format!("Bind world FXAA shader: {error}"))?;
        let fxaa_constants = [
            1.0 / resources.output_width as f32,
            1.0 / resources.output_height as f32,
            0.75,
            0.0312,
        ];
        unsafe { device.SetPixelShaderConstantF(0, fxaa_constants.as_ptr(), 1) }
            .map_err(|error| format!("Set world FXAA constants: {error}"))?;
        draw_fullscreen_quad(device, resources.output_width, resources.output_height)
    })();

    unsafe { device.SetTexture(0, None::<&IDirect3DBaseTexture9>) }.ok();
    let state_result = unsafe { post_world_state.Apply() }
        .map_err(|error| format!("Restore post-world D3D9 state: {error}"));
    let restore_result = restore_world_frame(device, state);

    draw_result?;
    state_result?;
    restore_result
}

fn restore_world_frame(device: &IDirect3DDevice9, state: &WorldFrameState) -> Result<(), String> {
    let detach_result = unsafe { device.SetDepthStencilSurface(None::<&IDirect3DSurface9>) }
        .map_err(|error| format!("Detach low-resolution world depth: {error}"));
    let color_result = unsafe { device.SetRenderTarget(0, &state.native_color) }
        .map_err(|error| format!("Restore native world color: {error}"));
    let depth_result = unsafe { device.SetDepthStencilSurface(&state.native_depth) }
        .map_err(|error| format!("Restore native world depth: {error}"));
    let viewport_result = unsafe { device.SetViewport(&state.native_viewport) }
        .map_err(|error| format!("Restore native world viewport: {error}"));
    let scissor_result = unsafe { device.SetScissorRect(&state.native_scissor) }
        .map_err(|error| format!("Restore native world scissor: {error}"));
    let scissor_state_result =
        unsafe { device.SetRenderState(D3DRS_SCISSORTESTENABLE, state.native_scissor_enabled) }
            .map_err(|error| format!("Restore native world scissor state: {error}"));

    detach_result?;
    color_result?;
    depth_result?;
    viewport_result?;
    scissor_result?;
    scissor_state_result
}

impl SinglePassResources {
    fn create(
        device: &IDirect3DDevice9,
        output_width: u32,
        output_height: u32,
        color_format: D3DFORMAT,
        scale: f32,
    ) -> Result<Self, String> {
        let input_width = ((output_width as f32 * scale).round() as u32).max(1);
        let input_height = ((output_height as f32 * scale).round() as u32).max(1);
        let source_texture =
            create_render_texture(device, input_width, input_height, color_format)?;
        let source_surface = unsafe { source_texture.GetSurfaceLevel(0) }
            .map_err(|error| format!("Get single-pass source surface: {error}"))?;
        let easu_shader = compile_shader(device, shaders::EASU, "single-pass FSR EASU")?;
        config::log(&format!(
            "Source resource created: {}x{} -> {}x{}",
            input_width, input_height, output_width, output_height
        ));

        Ok(Self {
            input_width,
            input_height,
            output_width,
            output_height,
            source_texture,
            source_surface,
            easu_shader,
        })
    }
}

impl Resources {
    fn create(
        device: &IDirect3DDevice9,
        input_width: u32,
        input_height: u32,
        output_width: u32,
        output_height: u32,
        color_format: D3DFORMAT,
    ) -> Result<Self, String> {
        let source_texture =
            create_render_texture(device, input_width, input_height, color_format)?;
        let source_surface = unsafe { source_texture.GetSurfaceLevel(0) }
            .map_err(|error| format!("Get source surface: {error}"))?;
        let easu_texture =
            create_render_texture(device, output_width, output_height, color_format)?;
        let easu_surface = unsafe { easu_texture.GetSurfaceLevel(0) }
            .map_err(|error| format!("Get EASU output surface: {error}"))?;
        let easu_shader = compile_shader(device, shaders::EASU, "FSR EASU")?;
        let rcas_shader = compile_shader(device, shaders::RCAS, "FSR RCAS")?;

        Ok(Self {
            input_width,
            input_height,
            output_width,
            output_height,
            color_format,
            source_texture,
            source_surface,
            easu_texture,
            easu_surface,
            easu_shader,
            rcas_shader,
        })
    }
}

fn create_render_texture(
    device: &IDirect3DDevice9,
    width: u32,
    height: u32,
    format: D3DFORMAT,
) -> Result<IDirect3DTexture9, String> {
    let mut texture = None;
    unsafe {
        device.CreateTexture(
            width,
            height,
            1,
            D3DUSAGE_RENDERTARGET as u32,
            format,
            D3DPOOL_DEFAULT,
            &mut texture,
            null_mut::<HANDLE>(),
        )
    }
    .map_err(|error| format!("Create {width}x{height} render texture: {error}"))?;
    texture.ok_or_else(|| "CreateTexture returned null".to_owned())
}

fn create_depth_surface(
    device: &IDirect3DDevice9,
    width: u32,
    height: u32,
    format: D3DFORMAT,
) -> Result<IDirect3DSurface9, String> {
    let mut surface = None;
    unsafe {
        device.CreateDepthStencilSurface(
            width,
            height,
            format,
            D3DMULTISAMPLE_NONE,
            0,
            true,
            &mut surface,
            null_mut::<HANDLE>(),
        )
    }
    .map_err(|error| format!("Create {width}x{height} depth surface: {error}"))?;
    surface.ok_or_else(|| "CreateDepthStencilSurface returned null".to_owned())
}

fn compile_shader(
    device: &IDirect3DDevice9,
    source: &str,
    display_name: &str,
) -> Result<IDirect3DPixelShader9, String> {
    let code = compile_shader_bytecode(source, display_name)?;
    unsafe { device.CreatePixelShader(code.GetBufferPointer().cast::<u32>()) }
        .map_err(|error| format!("Create {display_name} shader: {error}"))
}

fn compile_shader_bytecode(source: &str, display_name: &str) -> Result<ID3DBlob, String> {
    let mut code: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    let result = unsafe {
        D3DCompile(
            source.as_ptr().cast::<c_void>(),
            source.len(),
            PCSTR::null(),
            None,
            None::<&ID3DInclude>,
            s!("main"),
            s!("ps_3_0"),
            D3DCOMPILE_OPTIMIZATION_LEVEL3,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    if let Err(error) = result {
        let details = errors
            .as_ref()
            .map(blob_text)
            .filter(|message| !message.is_empty())
            .unwrap_or_else(|| error.to_string());
        return Err(format!("Compile {display_name}: {details}"));
    }

    code.ok_or_else(|| format!("Compile {display_name} returned no bytecode"))
}

fn blob_text(blob: &ID3DBlob) -> String {
    let bytes = unsafe {
        std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize())
    };
    String::from_utf8_lossy(bytes)
        .trim_end_matches('\0')
        .trim()
        .to_owned()
}

fn set_output_viewport(device: &IDirect3DDevice9, width: u32, height: u32) -> Result<(), String> {
    let viewport = D3DVIEWPORT9 {
        X: 0,
        Y: 0,
        Width: width,
        Height: height,
        MinZ: 0.0,
        MaxZ: 1.0,
    };
    unsafe { device.SetViewport(&viewport) }
        .map_err(|error| format!("Set output viewport: {error}"))
}

fn set_fullscreen_state(device: &IDirect3DDevice9) -> Result<(), String> {
    unsafe { device.SetVertexShader(None::<&IDirect3DVertexShader9>) }
        .map_err(|error| format!("Disable vertex shader: {error}"))?;
    unsafe { device.SetFVF(D3DFVF_XYZRHW | D3DFVF_TEX1) }
        .map_err(|error| format!("Set fullscreen FVF: {error}"))?;
    unsafe { device.SetRenderState(D3DRS_ZENABLE, 0) }.ok();
    unsafe { device.SetRenderState(D3DRS_ZWRITEENABLE, 0) }.ok();
    unsafe { device.SetRenderState(D3DRS_ALPHABLENDENABLE, 0) }.ok();
    unsafe { device.SetRenderState(D3DRS_ALPHATESTENABLE, 0) }.ok();
    unsafe { device.SetRenderState(D3DRS_CULLMODE, D3DCULL_NONE.0 as u32) }.ok();
    unsafe { device.SetRenderState(D3DRS_SCISSORTESTENABLE, 0) }.ok();
    unsafe { device.SetRenderState(D3DRS_COLORWRITEENABLE, 0xF) }.ok();
    unsafe { device.SetSamplerState(0, D3DSAMP_ADDRESSU, D3DTADDRESS_CLAMP.0 as u32) }.ok();
    unsafe { device.SetSamplerState(0, D3DSAMP_ADDRESSV, D3DTADDRESS_CLAMP.0 as u32) }.ok();
    unsafe { device.SetSamplerState(0, D3DSAMP_MINFILTER, D3DTEXF_POINT.0 as u32) }.ok();
    unsafe { device.SetSamplerState(0, D3DSAMP_MAGFILTER, D3DTEXF_POINT.0 as u32) }.ok();
    unsafe { device.SetSamplerState(0, D3DSAMP_MIPFILTER, D3DTEXF_NONE.0 as u32) }.ok();
    Ok(())
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    x: f32,
    y: f32,
    z: f32,
    rhw: f32,
    u: f32,
    v: f32,
}

fn draw_fullscreen_quad(device: &IDirect3DDevice9, width: u32, height: u32) -> Result<(), String> {
    let left = -0.5;
    let top = -0.5;
    let right = width as f32 - 0.5;
    let bottom = height as f32 - 0.5;
    let vertices = [
        Vertex {
            x: left,
            y: top,
            z: 0.0,
            rhw: 1.0,
            u: 0.0,
            v: 0.0,
        },
        Vertex {
            x: right,
            y: top,
            z: 0.0,
            rhw: 1.0,
            u: 1.0,
            v: 0.0,
        },
        Vertex {
            x: left,
            y: bottom,
            z: 0.0,
            rhw: 1.0,
            u: 0.0,
            v: 1.0,
        },
        Vertex {
            x: right,
            y: bottom,
            z: 0.0,
            rhw: 1.0,
            u: 1.0,
            v: 1.0,
        },
    ];

    unsafe {
        device.DrawPrimitiveUP(
            D3DPT_TRIANGLESTRIP,
            2,
            vertices.as_ptr().cast::<c_void>(),
            size_of::<Vertex>() as u32,
        )
    }
    .map_err(|error| format!("Draw fullscreen quad: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_all_world_post_process_combinations() {
        assert_eq!(
            world_pipeline_plan(false, false),
            WorldPipelinePlan {
                easu_to_intermediate: false,
                apply_rcas: false,
                rcas_to_intermediate: false,
                apply_fxaa: false,
                fxaa_from_rcas: false,
            }
        );
        assert_eq!(
            world_pipeline_plan(true, false),
            WorldPipelinePlan {
                easu_to_intermediate: true,
                apply_rcas: true,
                rcas_to_intermediate: false,
                apply_fxaa: false,
                fxaa_from_rcas: false,
            }
        );
        assert_eq!(
            world_pipeline_plan(false, true),
            WorldPipelinePlan {
                easu_to_intermediate: true,
                apply_rcas: false,
                rcas_to_intermediate: false,
                apply_fxaa: true,
                fxaa_from_rcas: false,
            }
        );
        assert_eq!(
            world_pipeline_plan(true, true),
            WorldPipelinePlan {
                easu_to_intermediate: true,
                apply_rcas: true,
                rcas_to_intermediate: true,
                apply_fxaa: true,
                fxaa_from_rcas: true,
            }
        );
    }
}
