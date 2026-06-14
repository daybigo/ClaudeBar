//! Genera dinamicamente el icono de la bandeja con el % de la sesion dibujado.
//! En Windows la bandeja no muestra texto al lado del icono (como en macOS),
//! asi que "pintamos" el numero dentro del propio icono.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use std::sync::OnceLock;
use tauri::image::Image;

const SIZE: u32 = 32;

static FONT: OnceLock<Option<FontVec>> = OnceLock::new();

fn font() -> Option<&'static FontVec> {
    FONT.get_or_init(|| {
        // Fuentes del sistema de Windows (negrita para legibilidad pequena).
        let candidates = [
            r"C:\Windows\Fonts\segoeuib.ttf",
            r"C:\Windows\Fonts\arialbd.ttf",
            r"C:\Windows\Fonts\seguisb.ttf",
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\arial.ttf",
        ];
        for c in candidates {
            if let Ok(bytes) = std::fs::read(c) {
                if let Ok(f) = FontVec::try_from_vec(bytes) {
                    return Some(f);
                }
            }
        }
        None
    })
    .as_ref()
}

#[inline]
fn blend(buf: &mut [u8], x: i32, y: i32, color: [u8; 3], alpha: f32) {
    if x < 0 || y < 0 || x >= SIZE as i32 || y >= SIZE as i32 || alpha <= 0.0 {
        return;
    }
    let a = alpha.clamp(0.0, 1.0);
    let idx = ((y as u32 * SIZE + x as u32) * 4) as usize;
    for k in 0..3 {
        let bg = buf[idx + k] as f32;
        let fg = color[k] as f32;
        buf[idx + k] = (fg * a + bg * (1.0 - a)) as u8;
    }
    let cur_a = buf[idx + 3] as f32 / 255.0;
    let new_a = a + cur_a * (1.0 - a);
    buf[idx + 3] = (new_a * 255.0) as u8;
}

/// Rellena un rectangulo redondeado que cubre todo el icono.
fn fill_rounded(buf: &mut [u8], color: [u8; 3], radius: f32) {
    let r = radius;
    let w = SIZE as f32;
    let h = SIZE as f32;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            // distancia a la esquina redondeada mas cercana
            let dx = if fx < r {
                r - fx
            } else if fx > w - r {
                fx - (w - r)
            } else {
                0.0
            };
            let dy = if fy < r {
                r - fy
            } else if fy > h - r {
                fy - (h - r)
            } else {
                0.0
            };
            let dist = (dx * dx + dy * dy).sqrt();
            // antialias de 1px en el borde
            let cov = (r - dist + 0.5).clamp(0.0, 1.0);
            blend(buf, x as i32, y as i32, color, cov);
        }
    }
}

/// Color del fondo segun el nivel de uso.
fn severity_color(util: f64) -> [u8; 3] {
    if util >= 90.0 {
        [222, 73, 65] // rojo
    } else if util >= 70.0 {
        [230, 145, 56] // naranja (como las barras del repo)
    } else if util >= 40.0 {
        [225, 178, 70] // ambar
    } else {
        [70, 140, 120] // verde calmado
    }
}

fn draw_text(buf: &mut [u8], text: &str, font: &FontVec) {
    let max_w = SIZE as f32 - 4.0;
    // Tamano base; se reduce si el texto es muy ancho (ej "100").
    let mut px = 22.0_f32;
    for _ in 0..6 {
        let scaled = font.as_scaled(PxScale::from(px));
        let total: f32 = text.chars().map(|c| scaled.h_advance(font.glyph_id(c))).sum();
        if total <= max_w {
            break;
        }
        px *= max_w / total;
    }

    let scaled = font.as_scaled(PxScale::from(px));
    let total_w: f32 = text.chars().map(|c| scaled.h_advance(font.glyph_id(c))).sum();
    let ascent = scaled.ascent();
    let descent = scaled.descent();
    let text_h = ascent - descent;
    let baseline_y = (SIZE as f32 - text_h) / 2.0 + ascent;
    let mut pen_x = (SIZE as f32 - total_w) / 2.0;

    for c in text.chars() {
        let id = font.glyph_id(c);
        let glyph = id.with_scale_and_position(PxScale::from(px), ab_glyph::point(pen_x, baseline_y));
        if let Some(outline) = font.outline_glyph(glyph) {
            let bb = outline.px_bounds();
            outline.draw(|gx, gy, cov| {
                let x = bb.min.x as i32 + gx as i32;
                let y = bb.min.y as i32 + gy as i32;
                blend(buf, x, y, [255, 255, 255], cov);
            });
        }
        pen_x += scaled.h_advance(id);
    }
}

/// Crea el icono. `percent` = utilizacion de la sesion (0-100) o None si no hay
/// datos todavia.
pub fn render(percent: Option<f64>) -> Image<'static> {
    let mut buf = vec![0u8; (SIZE * SIZE * 4) as usize];

    match percent {
        Some(p) => {
            let p = p.clamp(0.0, 100.0);
            fill_rounded(&mut buf, severity_color(p), 7.0);
            if let Some(f) = font() {
                draw_text(&mut buf, &format!("{}", p.round() as i64), f);
            }
        }
        None => {
            // Sin datos: punto gris tenue.
            fill_rounded(&mut buf, [120, 120, 130], 7.0);
            if let Some(f) = font() {
                draw_text(&mut buf, "–", f);
            }
        }
    }

    Image::new_owned(buf, SIZE, SIZE)
}
