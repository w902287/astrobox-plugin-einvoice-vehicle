use std::io::Cursor;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use fontdue::{Font, FontSettings};
use fontdue::layout::{CoordinateSystem, Layout, LayoutSettings, TextStyle};

const PORTRAIT_W: usize = 190;
const PORTRAIT_H: usize = 480;
const LANDSCAPE_W: usize = 480;
const LANDSCAPE_H: usize = 190;
const FONT_BYTES: &[u8] = include_bytes!("../assets/barcode-font.ttf");
const EINVOICE_LOGO: &[u8] = include_bytes!("../assets/einvoice-logo.png");
const OPENPOINT_LOGO: &[u8] = include_bytes!("../assets/openpoint-logo.png");
const FAMILY_LOGO: &[u8] = include_bytes!("../assets/familymart-logo.png");

const CODE128: [&str; 107] = [
"11011001100","11001101100","11001100110","10010011000","10010001100","10001001100","10011001000","10011000100","10001100100","11001001000","11001000100","11000100100","10110011100","10011011100","10011001110","10111001100","10011101100","10011100110","11001110010","11001011100","11001001110","11011100100","11001110100","11101101110","11101001100","11100101100","11100100110","11101100100","11100110100","11100110010","11011011000","11011000110","11000110110","10100011000","10001011000","10001000110","10110001000","10001101000","10001100010","11010001000","11000101000","11000100010","10110111000","10110001110","10001101110","10111011000","10111000110","10001110110","11101110110","11010001110","11000101110","11011101000","11011100010","11011101110","11101011000","11101000110","11100010110","11101101000","11101100010","11100011010","11101111010","11001000010","11110001010","10100110000","10100001100","10010110000","10010000110","10000101100","10000100110","10110010000","10110000100","10011010000","10011000010","10000110100","10000110010","11000010010","11001010000","11110111010","11000010100","10001111010","10100111100","10010111100","10010011110","10111100100","10011110100","10011110010","11110100100","11110010100","11110010010","11011011110","11011110110","11110110110","10101111000","10100011110","10001011110","10111101000","10111100010","11110101000","11110100010","10111011110","10111101110","11101011110","11110101110","11010000100","11010010000","11010011100","11000111010"
];

fn encode128(s: &str) -> Option<String> {
    if s.is_empty() || !s.bytes().all(|c| (32..=126).contains(&c)) { return None; }
    let b = s.as_bytes();
    let mut vals: Vec<usize> = Vec::new();
    let mut i = 0usize;
    let leading = b.iter().take_while(|c| c.is_ascii_digit()).count();
    let mut set_c = leading >= 4;
    vals.push(if set_c { 105 } else { 104 });
    while i < b.len() {
        let run = b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
        if !set_c && run >= 4 { vals.push(99); set_c = true; continue; }
        if set_c {
            if run >= 2 {
                vals.push(((b[i] - b'0') * 10 + (b[i + 1] - b'0')) as usize);
                i += 2;
                continue;
            }
            vals.push(100);
            set_c = false;
            continue;
        }
        vals.push((b[i] - 32) as usize);
        i += 1;
    }
    let sum = vals[0] + vals.iter().enumerate().skip(1).map(|(n, v)| n * v).sum::<usize>();
    vals.push(sum % 103);
    let mut bits = String::from("0000000000");
    for v in vals { bits.push_str(CODE128[v]); }
    bits.push_str("1100011101011");
    bits.push_str("0000000000");
    Some(bits)
}

fn blend_black(img: &mut [u8], x: i32, y: i32, alpha: u8) {
    if x < 0 || y < 0 || x >= LANDSCAPE_W as i32 || y >= LANDSCAPE_H as i32 { return; }
    let p = (y as usize * LANDSCAPE_W + x as usize) * 4;
    let v = 255u16.saturating_sub(alpha as u16) as u8;
    img[p] = img[p].min(v);
    img[p + 1] = img[p + 1].min(v);
    img[p + 2] = img[p + 2].min(v);
    img[p + 3] = 255;
}

fn fill_black(img: &mut [u8], x: i32, y: i32, w: i32, h: i32) {
    for yy in y.max(0)..(y + h).min(LANDSCAPE_H as i32) {
        for xx in x.max(0)..(x + w).min(LANDSCAPE_W as i32) { blend_black(img, xx, yy, 255); }
    }
}

fn text_width(font: &Font, text: &str, size: f32) -> f32 {
    text.chars().map(|c| font.metrics(c, size).advance_width).sum()
}

fn draw_text_at(img: &mut [u8], font: &Font, text: &str, size: f32, x: f32, y: f32) {
    let fonts = [font];
    let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
    layout.reset(&LayoutSettings { x, y, ..LayoutSettings::default() });
    layout.append(&fonts, &TextStyle::new(text, size, 0));
    for glyph in layout.glyphs() {
        let (_, bitmap) = font.rasterize_config(glyph.key);
        for gy in 0..glyph.height {
            for gx in 0..glyph.width {
                let alpha = bitmap[gy * glyph.width + gx];
                if alpha > 0 {
                    blend_black(img, glyph.x.round() as i32 + gx as i32, glyph.y.round() as i32 + gy as i32, alpha);
                }
            }
        }
    }
}

fn decode_png(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let src = &buf[..info.buffer_size()];
    let mut rgba = Vec::with_capacity(info.width as usize * info.height as usize * 4);
    match info.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(src),
        png::ColorType::Rgb => for p in src.chunks_exact(3) { rgba.extend_from_slice(&[p[0], p[1], p[2], 255]); },
        png::ColorType::Grayscale => for &v in src { rgba.extend_from_slice(&[v, v, v, 255]); },
        png::ColorType::GrayscaleAlpha => for p in src.chunks_exact(2) { rgba.extend_from_slice(&[p[0], p[0], p[0], p[1]]); },
        _ => return Err("unsupported logo color type".into()),
    }
    Ok((info.width as usize, info.height as usize, rgba))
}

fn draw_logo_scaled(img: &mut [u8], bytes: &[u8], x: i32, y: i32, out_w: usize, out_h: usize) -> Result<(), String> {
    let (w, h, logo) = decode_png(bytes)?;
    for yy in 0..out_h {
        for xx in 0..out_w {
            let sx = xx * w / out_w;
            let sy = yy * h / out_h;
            let s = (sy * w + sx) * 4;
            let dx = x + xx as i32;
            let dy = y + yy as i32;
            if dx < 0 || dy < 0 || dx >= LANDSCAPE_W as i32 || dy >= LANDSCAPE_H as i32 { continue; }
            let d = (dy as usize * LANDSCAPE_W + dx as usize) * 4;
            let a = logo[s + 3] as u16;
            for c in 0..3 {
                img[d + c] = ((logo[s + c] as u16 * a + img[d + c] as u16 * (255 - a)) / 255) as u8;
            }
            img[d + 3] = 255;
        }
    }
    Ok(())
}

fn rotate_landscape_to_final(landscape: &[u8]) -> Vec<u8> {
    let mut proven = vec![255u8; PORTRAIT_W * PORTRAIT_H * 4];
    for y in 0..LANDSCAPE_H {
        for x in 0..LANDSCAPE_W {
            let src = (y * LANDSCAPE_W + x) * 4;
            let dst_x = LANDSCAPE_H - 1 - y;
            let dst_y = x;
            let dst = (dst_y * PORTRAIT_W + dst_x) * 4;
            proven[dst..dst + 4].copy_from_slice(&landscape[src..src + 4]);
        }
    }
    let mut portrait = vec![255u8; PORTRAIT_W * PORTRAIT_H * 4];
    for y in 0..PORTRAIT_H {
        for x in 0..PORTRAIT_W {
            let src = (y * PORTRAIT_W + x) * 4;
            let dst_x = PORTRAIT_W - 1 - x;
            let dst_y = PORTRAIT_H - 1 - y;
            let dst = (dst_y * PORTRAIT_W + dst_x) * 4;
            portrait[dst..dst + 4].copy_from_slice(&proven[src..src + 4]);
        }
    }
    portrait
}

fn draw_header(img: &mut [u8], font: &Font, card_index: usize) -> Result<(), String> {
    match card_index {
        1 => {
            // User-provided OPENPOINT mark, enlarged and centered in the 30 px header strip.
            draw_logo_scaled(img, OPENPOINT_LOGO, 137, 3, 206, 27)?;
        }
        2 => {
            // User-provided FamilyMart mark plus a larger 27 px Chinese heading.
            let title = "全家會員條碼";
            let title_w = text_width(font, title, 27.0).ceil() as i32;
            let start_x = (LANDSCAPE_W as i32 - 28 - 8 - title_w) / 2;
            draw_logo_scaled(img, FAMILY_LOGO, start_x, 1, 28, 28)?;
            // Keep a 3 px white gap before the proven barcode region at y=32.
            draw_text_at(img, font, title, 27.0, (start_x + 36) as f32, -4.0);
        }
        _ => {
            let title = "統一發票共通載具";
            let title_w = text_width(font, title, 27.0).ceil() as i32;
            let start_x = (LANDSCAPE_W as i32 - 30 - 8 - title_w) / 2;
            draw_logo_scaled(img, EINVOICE_LOGO, start_x, 1, 30, 30)?;
            // Keep a 3 px white gap before the proven barcode region at y=32.
            draw_text_at(img, font, title, 27.0, (start_x + 38) as f32, -4.0);
        }
    }
    Ok(())
}

pub fn render_png(value: &str, card_index: usize) -> Result<Vec<u8>, String> {
    let bits = encode128(value).ok_or_else(|| "invalid Code128 value".to_string())?;
    let font = Font::from_bytes(FONT_BYTES, FontSettings::default()).map_err(|e| e.to_string())?;
    let mut landscape = vec![255u8; LANDSCAPE_W * LANDSCAPE_H * 4];

    // Keep the exact barcode geometry already proven on the watch.
    let start_x = 18i32;
    let total_width = 444i32;
    let bar_y = 32i32;
    let bar_h = 134i32;
    let module = total_width as f32 / bits.len() as f32;
    let mut run: Option<usize> = None;
    for (i, c) in bits.bytes().enumerate() {
        if c == b'1' && run.is_none() { run = Some(i); }
        if c == b'0' {
            if let Some(st) = run.take() {
                let x0 = start_x + (st as f32 * module).round() as i32;
                let x1 = start_x + (i as f32 * module).round() as i32;
                fill_black(&mut landscape, x0, bar_y, (x1 - x0).max(1), bar_h);
            }
        }
    }
    if let Some(st) = run {
        let x0 = start_x + (st as f32 * module).round() as i32;
        fill_black(&mut landscape, x0, bar_y, start_x + total_width - x0, bar_h);
    }

    draw_header(&mut landscape, &font, card_index)?;
    let label = match card_index {
        1 => format!("OPENPOINT：{value}"),
        2 => format!("全家會員：{value}"),
        _ => format!("手機條碼：{value}"),
    };
    let label_size = if label.chars().count() > 23 { 17.0 } else { 20.0 };
    let label_x = ((LANDSCAPE_W as f32 - text_width(&font, &label, label_size)) / 2.0).max(6.0);
    draw_text_at(&mut landscape, &font, &label, label_size, label_x, 166.0);

    let rgba = rotate_landscape_to_final(&landscape);
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(Cursor::new(&mut out), PORTRAIT_W as u32, PORTRAIT_H as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

pub fn render_base64(value: &str, card_index: usize) -> Result<String, String> {
    Ok(STANDARD.encode(render_png(value, card_index)?))
}
