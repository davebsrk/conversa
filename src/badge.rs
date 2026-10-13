//! Insignia con el número de chats sin leer (círculo rojo con el número en blanco), dibujada con
//! la fuente del sistema. Windows no tiene contadores nativos para apps no empaquetadas: el
//! número se pone como icono superpuesto de la barra de tareas y sobre el icono de la bandeja.

use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS,
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CENTER,
    DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, FW_BOLD, GdiFlush,
    OUT_DEFAULT_PRECIS, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::core::w;

const RED: [f32; 3] = [229.0, 56.0, 59.0];

/// Texto de la insignia: "1".."99" y "99+" a partir de 100.
pub fn label(count: u32) -> String {
    if count > 99 {
        "99+".into()
    } else {
        count.to_string()
    }
}

/// RGBA de `size`×`size` píxeles.
pub fn render(count: u32, size: u32) -> Vec<u8> {
    let text = label(count);
    let coverage = text_coverage(&text, size);
    let center = size as f32 / 2.0;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for i in 0..size * size {
        let (x, y) = ((i % size) as f32 + 0.5, (i / size) as f32 + 0.5);
        let distance = ((x - center).powi(2) + (y - center).powi(2)).sqrt();
        // Borde suavizado de un píxel.
        let circle = (center - distance).clamp(0.0, 1.0);
        let ink = coverage[i as usize];
        let channel = |red: f32| (red * (1.0 - ink) + 255.0 * ink) as u8;
        rgba.extend([
            channel(RED[0]),
            channel(RED[1]),
            channel(RED[2]),
            (circle * 255.0) as u8,
        ]);
    }
    rgba
}

/// Pinta `rgba` (la insignia) encima de `base` en su esquina superior derecha.
pub fn overlay(base: &mut [u8], base_size: u32, badge: &[u8], badge_size: u32) {
    let offset_x = base_size - badge_size;
    for y in 0..badge_size {
        for x in 0..badge_size {
            let src = ((y * badge_size + x) * 4) as usize;
            let dst = ((y * base_size + offset_x + x) * 4) as usize;
            let alpha = badge[src + 3] as f32 / 255.0;
            for c in 0..3 {
                base[dst + c] =
                    (badge[src + c] as f32 * alpha + base[dst + c] as f32 * (1.0 - alpha)) as u8;
            }
            base[dst + 3] = base[dst + 3].max(badge[src + 3]);
        }
    }
}

/// Cobertura (0..1) de cada píxel del texto, centrado, en Segoe UI negrita.
fn text_coverage(text: &str, size: u32) -> Vec<f32> {
    let pixels = (size * size) as usize;
    // Más pequeño cuantos más caracteres, para que quepa en el círculo.
    let height = (size as f32 * [0.78, 0.66, 0.5][text.len().clamp(1, 3) - 1]) as i32;
    unsafe {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size as i32,
                biHeight: -(size as i32), // filas de arriba abajo
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let dc = CreateCompatibleDC(None);
        let mut bits = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        else {
            let _ = DeleteDC(dc);
            return vec![0.0; pixels];
        };
        let font = CreateFontW(
            -height,
            0,
            0,
            0,
            FW_BOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            0,
            w!("Segoe UI"),
        );
        let old_bitmap = SelectObject(dc, bitmap.into());
        let old_font = SelectObject(dc, font.into());
        SetTextColor(dc, COLORREF(0x00ff_ffff));
        SetBkMode(dc, TRANSPARENT);
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: size as i32,
            bottom: size as i32,
        };
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        DrawTextW(
            dc,
            &mut wide,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        let _ = GdiFlush();

        // Texto blanco sobre negro: el brillo de cada píxel es su cobertura.
        let bgra = std::slice::from_raw_parts(bits as *const u8, pixels * 4);
        let coverage = bgra
            .chunks_exact(4)
            .map(|px| px[0].max(px[1]).max(px[2]) as f32 / 255.0)
            .collect();

        SelectObject(dc, old_font);
        SelectObject(dc, old_bitmap);
        let _ = DeleteObject(font.into());
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        coverage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_label_at_99() {
        assert_eq!(label(1), "1");
        assert_eq!(label(99), "99");
        assert_eq!(label(100), "99+");
    }

    #[test]
    fn draws_white_text_inside_red_circle() {
        let rgba = render(8, 32);
        let pixel = |x: u32, y: u32| &rgba[((y * 32 + x) * 4) as usize..][..4];
        assert_eq!(pixel(0, 0)[3], 0, "las esquinas son transparentes");
        assert!(
            rgba.chunks_exact(4).any(|px| px[3] == 255 && px[1] > 200),
            "hay texto blanco"
        );
        assert!(
            rgba.chunks_exact(4).any(|px| px[3] == 255 && px[1] < 80),
            "hay fondo rojo"
        );
    }
}
