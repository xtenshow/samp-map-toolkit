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
    let Some(status) = dynlod::overlay_status() else { return Ok(()); };
    let back_buffer = unsafe { device.GetBackBuffer(0, 0, D3DBACKBUFFER_TYPE_MONO) }
        .map_err(|e| format!("Overlay get backbuffer: {e}"))?;
    let mut desc = D3DSURFACE_DESC::default();
    unsafe { back_buffer.GetDesc(&mut desc) }
        .map_err(|e| format!("Overlay get backbuffer description: {e}"))?;
    let output_width = desc.Width;
    let output_height = desc.Height;
    if output_width == 0 || output_height == 0 { return Ok(()); }

    let active = renderer::active_world_render_dimensions();
    let (render_width, render_height, fsr_active) = match active {
        Some((w, h, _, _)) => (w, h, true),
        None => (output_width, output_height, false),
    };
    let scale = if fsr_active {
        ((render_width as f32 / output_width as f32)
            .min(render_height as f32 / output_height as f32) * 100.0).round() as u32
    } else { 100 };
    let fps = status.fps.map(|v| format!("{v:.0}")).unwrap_or_else(|| "--".to_owned());
    let line1 = format!("DYNLOD {} FPS {} TARGET {}", if status.enabled { "ON" } else { "OFF" }, fps, status.target.round() as u32);
    let line2 = format!("RENDER {}X{} OUT {}X{}", render_width, render_height, output_width, output_height);
    let line3 = format!("FSR {} SCALE {}%", if fsr_active { "ON" } else { "OFF" }, scale);

    let pixel = (output_width / 900).clamp(1, 3) as f32;
    let pad = 8.0 * pixel;
    let row_height = 7.0 * pixel;
    let row_gap = 3.0 * pixel;
    let lines = [&line1, &line2, &line3];
    let max_chars = lines.iter().map(|x| x.len()).max().unwrap_or(0);
    let panel_width = pad * 2.0 + max_chars as f32 * 6.0 * pixel;
    let panel_height = pad * 2.0 + row_height * 3.0 + row_gap * 2.0;
    let mut vertices = Vec::with_capacity(6 * (1 + lines.iter().map(|x| x.len() * 35).sum::<usize>()));
    rect(&mut vertices, 14.0, 14.0, 14.0 + panel_width, 14.0 + panel_height, 0xD018202C);
    for (i, line) in lines.iter().enumerate() {
        text(&mut vertices, line, 14.0 + pad, 14.0 + pad + i as f32 * (row_height + row_gap), pixel, 0xFFF0F4FA);
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
        unsafe { device.SetTextureStageState(0, D3DTSS_COLORARG1, D3DTA_DIFFUSE.0 as u32) }
            .map_err(|e| format!("Overlay set color argument: {e}"))?;
        unsafe { device.SetTextureStageState(0, D3DTSS_ALPHAOP, D3DTOP_SELECTARG1.0 as u32) }
            .map_err(|e| format!("Overlay set alpha operation: {e}"))?;
        unsafe { device.SetTextureStageState(0, D3DTSS_ALPHAARG1, D3DTA_DIFFUSE.0 as u32) }
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
        b' ' => [0;7], _ => [0;7],
    }
}
