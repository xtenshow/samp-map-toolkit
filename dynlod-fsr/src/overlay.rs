//! Small Direct3D 9 diagnostic HUD for the DynLOD/FSR integration.
use std::mem::size_of;
use windows::Win32::Graphics::Direct3D9::*;

use crate::{dynlod, renderer};

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    x: f32,
    y: f32,
    z: f32,
    rhw: f32,
    color: u32,
}

pub fn draw(device: &IDirect3DDevice9) -> Result<(), String> {
    let back_buffer = unsafe { device.GetBackBuffer(0, 0, D3DBACKBUFFER_TYPE_MONO) }
        .map_err(|e| format!("Overlay get backbuffer: {e}"))?;
    let mut desc = D3DSURFACE_DESC::default();
    unsafe { back_buffer.GetDesc(&mut desc) }
        .map_err(|e| format!("Overlay get backbuffer description: {e}"))?;
    let output_width = desc.Width;
    let output_height = desc.Height;
    if output_width == 0 || output_height == 0 { return Ok(()); }

    // The SA-MP menu receives input and gets painted on the same final frame.
    crate::dyn_menu::on_frame(output_width, output_height);
    let menu_view = crate::dyn_menu::view();
    let status = dynlod::overlay_status();
    if status.is_none() && menu_view.is_none() { return Ok(()); }

    let mut vertices = Vec::with_capacity(48_000);
    if let Some(status) = status {
        let active = renderer::active_world_render_dimensions();
        let (render_width, render_height, fsr_active) = match active {
            Some((w, h, _, _)) => (w, h, true),
            None => (output_width, output_height, false),
        };
        let scale = if fsr_active {
            ((render_width as f32 / output_width as f32)
                .min(render_height as f32 / output_height as f32) * 100.0).round() as u32
        } else { 100 };
        let fmt = |value: Option<f32>| value.map(|v| format!("{v:.0}")).unwrap_or_else(|| "N/A".to_owned());
        let fps = fmt(status.fps);
        let avg = fmt(status.avg_fps);
        let cpu = status.cpu_pct.map(|v| format!("{v:.0}%")).unwrap_or_else(|| "N/A".to_owned());
        let gpu = status.gpu_pct.map(|v| format!("{v:.0}%")).unwrap_or_else(|| "N/A".to_owned());
        let ram = status.ram_mb.map(|v| format!("{v}MB")).unwrap_or_else(|| "N/A".to_owned());

        let lines = [
            format!("DYNLOD {} FPS {} TARGET {}", if status.enabled { "ON" } else { "OFF" }, fps, status.target.round() as u32),
            format!("AVG FPS {} CPU {}", avg, cpu),
            format!("RAM {} GPU {}", ram, gpu),
            format!("RENDER {}X{} OUT {}X{}", render_width, render_height, output_width, output_height),
            format!("FSR {} SCALE {}%", if fsr_active { "ON" } else { "OFF" }, scale),
        ];
        let pixel = (output_width / 900).clamp(1, 3) as f32;
        let pad = 8.0 * pixel;
        let row_height = 7.0 * pixel;
        let row_gap = 3.0 * pixel;
        let max_chars = lines.iter().map(|x| x.len()).max().unwrap_or(0);
        let panel_width = pad * 2.0 + max_chars as f32 * 6.0 * pixel;
        let panel_height = pad * 2.0 + row_height * lines.len() as f32 + row_gap * (lines.len() - 1) as f32;
        rect(&mut vertices, 14.0, 14.0, 14.0 + panel_width, 14.0 + panel_height, 0xD018202C);
        for (i, line) in lines.iter().enumerate() {
            text(&mut vertices, line, 14.0 + pad, 14.0 + pad + i as f32 * (row_height + row_gap), pixel, 0xFFF0F4FA);
        }
    }

    if let Some(view) = menu_view {
        draw_menu(&mut vertices, &view, output_width, output_height);
    }
    if vertices.is_empty() { return Ok(()); }

    let state = unsafe { device.CreateStateBlock(D3DSBT_ALL) }
        .map_err(|e| format!("Overlay create D3D state block: {e}"))?;
    unsafe { state.Capture() }.map_err(|e| format!("Overlay capture D3D state: {e}"))?;
    unsafe { device.BeginScene() }.map_err(|e| format!("Overlay BeginScene: {e}"))?;

    let draw_result = (|| -> Result<(), String> {
        unsafe { device.SetVertexShader(None::<&IDirect3DVertexShader9>) }
            .map_err(|e| format!("Overlay disable vertex shader: {e}"))?;
        unsafe { device.SetPixelShader(None::<&IDirect3DPixelShader9>) }
            .map_err(|e| format!("Overlay disable pixel shader: {e}"))?;
        unsafe { device.SetTexture(0, None::<&IDirect3DBaseTexture9>) }
            .map_err(|e| format!("Overlay clear texture: {e}"))?;
        unsafe { device.SetFVF(D3DFVF_XYZRHW | D3DFVF_DIFFUSE) }
            .map_err(|e| format!("Overlay set FVF: {e}"))?;
        for (state_name, value) in [
            (D3DRS_ZENABLE, 0),
            (D3DRS_ZWRITEENABLE, 0),
            (D3DRS_ALPHABLENDENABLE, 1),
            (D3DRS_ALPHATESTENABLE, 0),
            (D3DRS_LIGHTING, 0),
            (D3DRS_FOGENABLE, 0),
            (D3DRS_SCISSORTESTENABLE, 0),
            (D3DRS_CULLMODE, D3DCULL_NONE.0 as u32),
            (D3DRS_SRCBLEND, D3DBLEND_SRCALPHA.0 as u32),
            (D3DRS_DESTBLEND, D3DBLEND_INVSRCALPHA.0 as u32),
        ] {
            unsafe { device.SetRenderState(state_name, value) }
                .map_err(|e| format!("Overlay set render state: {e}"))?;
        }
        unsafe { device.SetTextureStageState(0, D3DTSS_COLOROP, D3DTOP_SELECTARG1.0 as u32) }
            .map_err(|e| format!("Overlay set color operation: {e}"))?;
        unsafe { device.SetTextureStageState(0, D3DTSS_COLORARG1, D3DTA_DIFFUSE) }
            .map_err(|e| format!("Overlay set color argument: {e}"))?;
        unsafe { device.SetTextureStageState(0, D3DTSS_ALPHAOP, D3DTOP_SELECTARG1.0 as u32) }
            .map_err(|e| format!("Overlay set alpha operation: {e}"))?;
        unsafe { device.SetTextureStageState(0, D3DTSS_ALPHAARG1, D3DTA_DIFFUSE) }
            .map_err(|e| format!("Overlay set alpha argument: {e}"))?;
        unsafe {
            device.DrawPrimitiveUP(
                D3DPT_TRIANGLELIST,
                (vertices.len() / 3) as u32,
                vertices.as_ptr().cast::<std::ffi::c_void>(),
                size_of::<Vertex>() as u32,
            )
        }.map_err(|e| format!("Overlay DrawPrimitiveUP: {e}"))?;
        Ok(())
    })();

    let restore_result = unsafe { state.Apply() }
        .map_err(|e| format!("Overlay restore D3D state: {e}"));
    let end_result = unsafe { device.EndScene() }
        .map_err(|e| format!("Overlay EndScene: {e}"));
    draw_result?;
    restore_result?;
    end_result?;
    Ok(())
}

fn draw_menu(
    vertices: &mut Vec<Vertex>,
    view: &crate::dyn_menu::MenuView,
    output_width: u32,
    output_height: u32,
) {
    let scale = (output_width as f32 / 1280.0).clamp(0.75, 1.8);
    let panel_width = 600.0 * scale;
    let panel_height = (112.0 + view.rows.len() as f32 * 28.0) * scale;
    let left = (output_width as f32 - panel_width) * 0.5;
    let top = (output_height as f32 - panel_height) * 0.5;
    rect(vertices, left, top, left + panel_width, top + panel_height, 0xEA101923);
    rect(vertices, left, top, left + panel_width, top + 3.0 * scale, 0xFF36A6E8);
    text(vertices, &view.title, left + 20.0 * scale, top + 17.0 * scale, 1.8 * scale, 0xFF63C5FF);
    text(vertices, &view.description, left + 20.0 * scale, top + 52.0 * scale, 1.0 * scale, 0xFFB5C7D8);

    let row_top = top + 96.0 * scale;
    let row_height = 28.0 * scale;
    for (index, row) in view.rows.iter().enumerate() {
        let y = row_top + index as f32 * row_height;
        if index == view.selected {
            rect(vertices, left + 12.0 * scale, y - 4.0 * scale, left + panel_width - 12.0 * scale, y + 21.0 * scale, 0xAA245A7A);
        }
        text(
            vertices,
            row,
            left + 24.0 * scale,
            y,
            1.25 * scale,
            if index == view.selected { 0xFFFFFFFF } else { 0xFFE2EAF2 },
        );
    }
}

fn rect(out: &mut Vec<Vertex>, left: f32, top: f32, right: f32, bottom: f32, color: u32) {
    let a = Vertex { x: left - 0.5, y: top - 0.5, z: 0.0, rhw: 1.0, color };
    let b = Vertex { x: right - 0.5, y: top - 0.5, z: 0.0, rhw: 1.0, color };
    let c = Vertex { x: right - 0.5, y: bottom - 0.5, z: 0.0, rhw: 1.0, color };
    let d = Vertex { x: left - 0.5, y: bottom - 0.5, z: 0.0, rhw: 1.0, color };
    out.extend_from_slice(&[a, b, c, a, c, d]);
}

fn text(out: &mut Vec<Vertex>, value: &str, x: f32, y: f32, scale: f32, color: u32) {
    let mut cursor = x;
    for ch in value.bytes() {
        let rows = glyph(ch);
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) != 0 {
                    let left = cursor + col as f32 * scale;
                    let top = y + row as f32 * scale;
                    rect(out, left, top, left + scale, top + scale, color);
                }
            }
        }
        cursor += 6.0 * scale;
    }
}

fn glyph(ch: u8) -> [u8; 7] {
    match ch {
        b'A' => [14,17,17,31,17,17,17], b'B' => [30,17,17,30,17,17,30],
        b'C' => [14,17,16,16,16,17,14], b'D' => [30,17,17,17,17,17,30],
        b'E' => [31,16,16,30,16,16,31], b'F' => [31,16,16,30,16,16,16],
        b'G' => [14,17,16,23,17,17,15], b'H' => [17,17,17,31,17,17,17],
        b'I' => [14,4,4,4,4,4,14], b'J' => [7,2,2,2,18,18,12],
        b'K' => [17,18,20,24,20,18,17], b'L' => [16,16,16,16,16,16,31],
        b'M' => [17,27,21,21,17,17,17], b'N' => [17,25,21,19,17,17,17],
        b'O' => [14,17,17,17,17,17,14], b'P' => [30,17,17,30,16,16,16],
        b'Q' => [14,17,17,17,21,18,13], b'R' => [30,17,17,30,20,18,17],
        b'S' => [15,16,16,14,1,1,30], b'T' => [31,4,4,4,4,4,4],
        b'U' => [17,17,17,17,17,17,14], b'V' => [17,17,17,17,17,10,4],
        b'W' => [17,17,17,21,21,21,10], b'X' => [17,17,10,4,10,17,17],
        b'Y' => [17,17,10,4,4,4,4], b'Z' => [31,1,2,4,8,16,31],
        b'0' => [14,17,19,21,25,17,14], b'1' => [4,12,4,4,4,4,14],
        b'2' => [14,17,1,2,4,8,31], b'3' => [30,1,1,14,1,1,30],
        b'4' => [2,6,10,18,31,2,2], b'5' => [31,16,16,30,1,1,30],
        b'6' => [14,16,16,30,17,17,14], b'7' => [31,1,2,4,8,8,8],
        b'8' => [14,17,17,14,17,17,14], b'9' => [14,17,17,15,1,1,14],
        b':' => [0,4,4,0,4,4,0], b'%' => [17,2,4,8,17,0,0],
        b'/' => [1,2,2,4,8,8,16], b'-' => [0,0,0,31,0,0,0],
        b'.' => [0,0,0,0,0,4,4],
        b' ' => [0;7], _ => [0;7],
    }
}
