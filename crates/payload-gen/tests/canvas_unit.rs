use std::f64::consts::TAU;

use core_utils::bench;
use payload_gen::{
    CANVAS_OP_FILL_RECT, CANVAS_OP_FILL_TEXT, CANVAS_OP_GET_IMAGE_DATA, CANVAS_OP_MEASURE,
    CANVAS_OP_TO_URL, CanvasRaster, Rgba8, bench_jitter, canvas_time_cost_us,
    parse_color, png_data_url_pixels,
};

fn px_at(px: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
    let i = (y * w + x) * 4;
    px[i..i + 4].try_into().unwrap()
}

fn decode_png(url: &str) -> Vec<u8> {
    let b64 = url.strip_prefix("data:image/png;base64,").expect("prefix");
    base64_turbo::STANDARD
        .decode(b64.as_bytes())
        .expect("base64")
}

fn walk_chunks(png: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    let mut out = Vec::new();
    let mut i = 8usize;
    while i + 12 <= png.len() {
        let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
        let tag: [u8; 4] = png[i + 4..i + 8].try_into().unwrap();
        let data = &png[i + 8..i + 8 + len];
        let crc = u32::from_be_bytes(png[i + 8 + len..i + 12 + len].try_into().unwrap());
        assert_eq!(crc, !core_utils::crc32_feed(core_utils::crc32_feed(0xFFFF_FFFF, &tag), data));
        out.push((tag, data.to_vec()));
        i += 12 + len;
    }
    assert_eq!(i, png.len());
    out
}

fn inflate_stored(z: &[u8]) -> Vec<u8> {
    assert_eq!(&z[..2], &[0x78, 0x01]);
    let mut i = 2usize;
    let mut raw = Vec::new();
    loop {
        let b = z[i];
        i += 1;
        assert_eq!(b & 0x06, 0);
        let len = u16::from_le_bytes([z[i], z[i + 1]]) as usize;
        i += 2;
        let nlen = u16::from_le_bytes([z[i], z[i + 1]]) as usize;
        i += 2;
        assert_eq!(!(len as u16) as usize, nlen);
        raw.extend_from_slice(&z[i..i + len]);
        i += len;
        if b & 1 == 1 {
            break;
        }
    }
    let adler = u32::from_be_bytes(z[i..i + 4].try_into().unwrap());
    assert_eq!(adler, core_utils::adler32_feed(1, &raw));
    assert_eq!(i + 4, z.len());
    raw
}

fn idat_of(chunks: &[([u8; 4], Vec<u8>)]) -> &[u8] {
    let (_, data) = chunks.iter().find(|(t, _)| t == b"IDAT").expect("idat");
    data
}

#[test]
fn png_structure_magic_ihdr_idat_iend() {
    let pixels: Vec<u8> = (0..8u32 * 4 * 4).map(|i| (i * 7 % 251) as u8).collect();
    let url = png_data_url_pixels(8, 4, &pixels);
    assert!(url.starts_with("data:image/png;base64,"));
    let png = decode_png(&url);
    assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    let chunks = walk_chunks(&png);
    assert_eq!(chunks.len(), 3);
    assert_eq!(&chunks[0].0, b"IHDR");
    assert_eq!(chunks[0].1.len(), 13);
    assert_eq!(&chunks[0].1[..4], &8u32.to_be_bytes());
    assert_eq!(&chunks[0].1[4..8], &4u32.to_be_bytes());
    assert_eq!(&chunks[0].1[8..13], &[8, 6, 0, 0, 0]);
    assert_eq!(&chunks[1].0, b"IDAT");
    assert_eq!(&chunks[2].0, b"IEND");
    assert!(chunks[2].1.is_empty());
}

#[test]
fn png_idat_stored_deflate_roundtrip_matches_pixels() {
    let (w, h) = (13u32, 7u32);
    let pixels: Vec<u8> = (0..w * h * 4).map(|i| ((i * 31 + 5) % 256) as u8).collect();
    let url = png_data_url_pixels(w, h, &pixels);
    let png = decode_png(&url);
    let raw = inflate_stored(idat_of(&walk_chunks(&png)));
    let stride = 1 + w as usize * 4;
    assert_eq!(raw.len(), h as usize * stride);
    for y in 0..h as usize {
        assert_eq!(raw[y * stride], 0);
        for x in 0..w as usize {
            let got = &raw[y * stride + 1 + x * 4..y * stride + 1 + x * 4 + 4];
            let want = &pixels[(y * w as usize + x) * 4..(y * w as usize + x) * 4 + 4];
            assert_eq!(got, want);
        }
    }
}

#[test]
fn png_splits_into_multiple_stored_blocks_when_large() {
    let (w, h) = (2000u32, 9u32);
    let pixels = vec![77u8; (w * h * 4) as usize];
    let url = png_data_url_pixels(w, h, &pixels);
    let png = decode_png(&url);
    let idat = idat_of(&walk_chunks(&png)).to_vec();
    let mut blocks = 0usize;
    let mut i = 2usize;
    loop {
        let b = idat[i];
        i += 1;
        let len = u16::from_le_bytes([idat[i], idat[i + 1]]) as usize;
        i += 2;
        let nlen = u16::from_le_bytes([idat[i], idat[i + 1]]) as usize;
        i += 2;
        assert_eq!(!(len as u16) as usize, nlen);
        assert!(len <= 65535);
        i += len;
        blocks += 1;
        if b & 1 == 1 {
            break;
        }
    }
    assert_eq!(blocks, 2);
    let raw = inflate_stored(&idat);
    assert_eq!(raw.len(), h as usize * (w as usize * 4 + 1));
}

#[test]
fn png_zero_dimensions_clamp_to_minimum() {
    for (w, h, ew, eh) in [(0u32, 0u32, 1u32, 1u32), (0, 5, 1, 5), (5, 0, 5, 1)] {
        let url = png_data_url_pixels(w, h, &[]);
        let png = decode_png(&url);
        let chunks = walk_chunks(&png);
        assert_eq!(&chunks[0].1[..4], &ew.to_be_bytes());
        assert_eq!(&chunks[0].1[4..8], &eh.to_be_bytes());
        let raw = inflate_stored(idat_of(&chunks));
        assert_eq!(raw.len(), eh as usize * (ew as usize * 4 + 1));
        assert!(raw.iter().all(|&b| b == 0));
    }
}

#[test]
fn png_data_url_pixels_is_pure() {
    let p = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
    let a = png_data_url_pixels(1, 2, &p);
    let b = png_data_url_pixels(1, 2, &p);
    assert_eq!(a, b);
    let q = vec![9u8, 9, 9, 9, 9, 9, 9, 9];
    assert_ne!(a, png_data_url_pixels(1, 2, &q));
}

#[test]
fn render_to_png_roundtrip_preserves_pixels() {
    let mut r = CanvasRaster::new(0xC0FFEE);
    r.set_fill(parse_color("#ff0000"));
    r.fill_rect(1.0, 1.0, 2.0, 2.0);
    let px = r.render(4, 4);
    let url = png_data_url_pixels(4, 4, &px);
    let png = decode_png(&url);
    let raw = inflate_stored(idat_of(&walk_chunks(&png)));
    for y in 0..4usize {
        assert_eq!(raw[y * 17], 0);
        assert_eq!(&raw[y * 17 + 1..y * 17 + 17], &px[y * 16..y * 16 + 16]);
    }
}

#[test]
fn empty_commands_render_all_transparent() {
    let r = CanvasRaster::new(0xABCD);
    let px = r.render(8, 8);
    assert_eq!(px.len(), 8 * 8 * 4);
    assert!(px.iter().all(|&b| b == 0));
    let px = CanvasRaster::new(7).render(0, 0);
    assert_eq!(px.len(), 4);
    assert!(px.iter().all(|&b| b == 0));
}

fn draw_scene(r: &mut CanvasRaster) {
    r.set_fill(parse_color("#123456"));
    r.fill_rect(0.0, 0.0, 64.0, 48.0);
    r.set_fill(parse_color("white"));
    r.begin_path();
    r.move_to(4.0, 4.0);
    r.line_to(20.0, 4.0);
    r.line_to(12.0, 18.0);
    r.close_path();
    r.fill();
    r.set_stroke(parse_color("cyan"));
    r.set_line_width(2.0);
    r.stroke_rect(30.0, 6.0, 10.0, 10.0);
    r.set_fill(parse_color("rgba(255,0,0,1)"));
    r.begin_path();
    r.arc(48.0, 24.0, 8.0, 0.0, TAU);
    r.fill();
    r.set_fill(parse_color("black"));
    r.fill_text("Zg", 6.0, 40.0);
}

#[test]
fn render_is_deterministic_across_calls_and_instances() {
    let mut a = CanvasRaster::new(0xFEED);
    draw_scene(&mut a);
    let mut b = CanvasRaster::new(0xFEED);
    draw_scene(&mut b);
    let first = a.render(64, 48);
    let second = a.render(64, 48);
    assert_eq!(first, second);
    assert_eq!(first, b.render(64, 48));
    let mut c = CanvasRaster::new(0xFEED + 1);
    draw_scene(&mut c);
    assert_ne!(first, c.render(64, 48));
}

#[test]
fn raster_output_depends_on_commands() {
    let mut a = CanvasRaster::new(3);
    a.set_fill(parse_color("red"));
    a.fill_rect(0.0, 0.0, 8.0, 8.0);
    let mut b = CanvasRaster::new(3);
    b.set_fill(parse_color("blue"));
    b.fill_rect(0.0, 0.0, 8.0, 8.0);
    assert_ne!(a.render(8, 8), b.render(8, 8));
}

#[test]
fn fill_rect_rasterizes_red_block() {
    let mut r = CanvasRaster::new(99);
    r.set_fill(parse_color("#ff0000"));
    r.fill_rect(2.0, 2.0, 4.0, 4.0);
    let px = r.render(16, 16);
    let c = px_at(&px, 16, 3, 3);
    assert_eq!(c[3], 255);
    assert!((c[0] as i32 - 255).abs() <= 1);
    assert!(c[1] <= 1 && c[2] <= 1);
    for (x, y) in [(1usize, 1usize), (7, 7), (0, 15), (15, 0)] {
        assert_eq!(px_at(&px, 16, x, y), [0, 0, 0, 0]);
    }
}

#[test]
fn farble_flips_five_percent_lsb_only() {
    let mut r = CanvasRaster::new(0x5EED);
    r.set_fill(parse_color("black"));
    r.fill_rect(0.0, 0.0, 200.0, 200.0);
    let px = r.render(200, 200);
    assert_eq!(px.len(), 200 * 200 * 4);
    let mut touched = 0usize;
    for p in px.chunks_exact(4) {
        assert_eq!(p[3], 255);
        assert!(p[0] <= 1 && p[1] <= 1 && p[2] <= 1);
        if p[0] | p[1] | p[2] != 0 {
            touched += 1;
        }
    }
    let frac = touched as f64 / 40000.0;
    assert!(
        (0.03..=0.07).contains(&frac),
        "farble fraction out of band: {frac}"
    );
}

#[test]
fn stroke_rect_draws_ring_not_interior() {
    let mut r = CanvasRaster::new(5);
    r.set_stroke(parse_color("white"));
    r.set_line_width(2.0);
    r.stroke_rect(2.0, 2.0, 8.0, 8.0);
    let px = r.render(16, 16);
    assert_eq!(px_at(&px, 16, 1, 5)[3], 255);
    assert_eq!(px_at(&px, 16, 10, 5)[3], 255);
    assert_eq!(px_at(&px, 16, 5, 5), [0, 0, 0, 0]);
}

#[test]
fn clear_rect_wipes_region_keeps_rest() {
    let mut r = CanvasRaster::new(6);
    r.set_fill(parse_color("blue"));
    r.fill_rect(0.0, 0.0, 16.0, 16.0);
    r.clear_rect(4.0, 4.0, 4.0, 4.0);
    let px = r.render(16, 16);
    assert_eq!(px_at(&px, 16, 5, 5), [0, 0, 0, 0]);
    assert_eq!(px_at(&px, 16, 2, 2)[3], 255);
}

#[test]
fn path_fill_triangle() {
    let mut r = CanvasRaster::new(7);
    r.set_fill(parse_color("green"));
    r.begin_path();
    r.move_to(1.0, 1.0);
    r.line_to(15.0, 1.0);
    r.line_to(8.0, 14.0);
    r.close_path();
    r.fill();
    let px = r.render(16, 16);
    assert_eq!(px_at(&px, 16, 8, 5)[3], 255);
    assert_eq!(px_at(&px, 16, 0, 5), [0, 0, 0, 0]);
    assert_eq!(px_at(&px, 16, 8, 15), [0, 0, 0, 0]);
}

#[test]
fn quadratic_curve_fill_shape() {
    let mut r = CanvasRaster::new(8);
    r.set_fill(parse_color("lime"));
    r.begin_path();
    r.move_to(0.0, 8.0);
    r.quadratic_curve_to(8.0, 0.0, 16.0, 8.0);
    r.close_path();
    r.fill();
    let px = r.render(16, 16);
    assert_eq!(px_at(&px, 16, 8, 6)[3], 255);
    assert_eq!(px_at(&px, 16, 8, 2), [0, 0, 0, 0]);
}

#[test]
fn bezier_curve_fill_shape() {
    let mut r = CanvasRaster::new(10);
    r.set_fill(parse_color("olive"));
    r.begin_path();
    r.move_to(0.0, 8.0);
    r.bezier_curve_to(5.5, 0.0, 10.5, 0.0, 16.0, 8.0);
    r.close_path();
    r.fill();
    let px = r.render(16, 16);
    assert_eq!(px_at(&px, 16, 8, 5)[3], 255);
    assert_eq!(px_at(&px, 16, 8, 0), [0, 0, 0, 0]);
}

#[test]
fn arc_full_and_half_circle_fill() {
    let mut r = CanvasRaster::new(12);
    r.set_fill(parse_color("navy"));
    r.begin_path();
    r.arc(8.0, 8.0, 6.0, 0.0, TAU);
    r.fill();
    let px = r.render(16, 16);
    assert_eq!(px_at(&px, 16, 8, 8)[3], 255);
    assert_eq!(px_at(&px, 16, 3, 8)[3], 255);
    assert_eq!(px_at(&px, 16, 0, 0), [0, 0, 0, 0]);

    let mut r = CanvasRaster::new(13);
    r.set_fill(parse_color("navy"));
    r.begin_path();
    r.arc(
        8.0,
        8.0,
        6.0,
        -std::f64::consts::FRAC_PI_2,
        std::f64::consts::FRAC_PI_2,
    );
    r.close_path();
    r.fill();
    let px = r.render(16, 16);
    assert_eq!(px_at(&px, 16, 12, 8)[3], 255);
    assert_eq!(px_at(&px, 16, 4, 8), [0, 0, 0, 0]);
}

#[test]
fn ellipse_fill_shape() {
    let mut r = CanvasRaster::new(14);
    r.set_fill(parse_color("teal"));
    r.begin_path();
    r.ellipse(8.0, 8.0, 6.0, 3.0, 0.0, 0.0, TAU);
    r.fill();
    let px = r.render(16, 16);
    assert_eq!(px_at(&px, 16, 8, 8)[3], 255);
    assert_eq!(px_at(&px, 16, 12, 8)[3], 255);
    assert_eq!(px_at(&px, 16, 8, 4), [0, 0, 0, 0]);
}

#[test]
fn fill_text_renders_deterministic_glyphs() {
    let mut r = CanvasRaster::new(0x7E57);
    r.set_fill(parse_color("black"));
    r.fill_text("Ag", 2.0, 9.0);
    let px = r.render(24, 16);
    let opaque = px.chunks_exact(4).filter(|p| p[3] > 0).count();
    assert!(opaque > 10, "glyph coverage too low: {opaque}");

    let mut r2 = CanvasRaster::new(0x7E57);
    r2.set_fill(parse_color("black"));
    r2.fill_text("Ag", 2.0, 9.0);
    assert_eq!(px, r2.render(24, 16));

    let mut r3 = CanvasRaster::new(0x7E57);
    r3.set_fill(parse_color("black"));
    r3.fill_text("AG", 2.0, 9.0);
    assert_ne!(px, r3.render(24, 16));
}

#[test]
fn fill_text_non_ascii_uses_stable_pseudo_glyph() {
    let mut a = CanvasRaster::new(21);
    a.fill_text("Ж", 2.0, 9.0);
    let pa = a.render(24, 16);
    let mut b = CanvasRaster::new(21);
    b.fill_text("Ж", 2.0, 9.0);
    assert_eq!(pa, b.render(24, 16));
    let mut c = CanvasRaster::new(21);
    c.fill_text("A", 2.0, 9.0);
    assert_ne!(pa, c.render(24, 16));
}

#[test]
fn stroke_text_renders_outlines() {
    let mut r = CanvasRaster::new(15);
    r.set_stroke(parse_color("white"));
    r.set_line_width(1.0);
    r.stroke_text("W", 2.0, 10.0);
    let px = r.render(20, 16);
    assert!(px.chunks_exact(4).any(|p| p[3] > 0));
}

#[test]
fn resize_resets_bitmap_and_state() {
    let mut r = CanvasRaster::new(4);
    r.set_fill(parse_color("red"));
    r.fill_rect(0.0, 0.0, 8.0, 8.0);
    r.fill_text("Q", 1.0, 5.0);
    r.resize();
    assert!(r.render(8, 8).iter().all(|&b| b == 0));
    r.fill_rect(0.0, 0.0, 8.0, 8.0);
    let px = r.render(8, 8);
    assert_eq!(px[3], 255);
    assert!(px[0] <= 1 && px[1] <= 1 && px[2] <= 1);
}

#[test]
fn global_alpha_scales_fill_and_ignores_invalid() {
    let mut r = CanvasRaster::new(9);
    r.set_fill(parse_color("red"));
    r.set_alpha(0.5);
    r.set_alpha(2.0);
    r.set_alpha(-0.5);
    r.set_alpha(f64::NAN);
    r.fill_rect(0.0, 0.0, 8.0, 8.0);
    let px = r.render(8, 8);
    let c = px_at(&px, 8, 0, 4);
    assert_eq!(c[3], 128);
    assert!((c[0] as i32 - 255).abs() <= 1);
    assert!(c[1] <= 1 && c[2] <= 1);

    let mut r2 = CanvasRaster::new(9);
    r2.set_fill(parse_color("red"));
    r2.set_alpha(0.0);
    r2.fill_rect(0.0, 0.0, 8.0, 8.0);
    assert!(r2.render(8, 8).iter().all(|&b| b == 0));
}

#[test]
fn line_width_invalid_values_are_ignored() {
    let mut r = CanvasRaster::new(11);
    r.set_stroke(parse_color("white"));
    r.set_line_width(0.0);
    r.set_line_width(-2.0);
    r.set_line_width(f64::NAN);
    r.stroke_rect(4.0, 4.0, 8.0, 8.0);
    let thin = r.render(16, 16);
    assert!(thin.chunks_exact(4).any(|p| p[3] > 0));

    let mut wide = CanvasRaster::new(11);
    wide.set_stroke(parse_color("white"));
    wide.set_line_width(4.0);
    wide.stroke_rect(4.0, 4.0, 8.0, 8.0);
    let thick = wide.render(16, 16);
    let cov = |p: &[u8]| p.chunks_exact(4).filter(|q| q[3] > 0).count();
    assert!(cov(&thick) > cov(&thin));
}

#[test]
fn parse_color_accepts_css_forms() {
    assert_eq!(
        parse_color("#f60"),
        Rgba8 {
            r: 255,
            g: 102,
            b: 0,
            a: 255
        }
    );
    assert_eq!(
        parse_color("#F60"),
        Rgba8 {
            r: 255,
            g: 102,
            b: 0,
            a: 255
        }
    );
    assert_eq!(
        parse_color("#ff6600"),
        Rgba8 {
            r: 255,
            g: 102,
            b: 0,
            a: 255
        }
    );
    assert_eq!(
        parse_color("#ff660080"),
        Rgba8 {
            r: 255,
            g: 102,
            b: 0,
            a: 128
        }
    );
    assert_eq!(
        parse_color("rgba(1,2,3,0.5)"),
        Rgba8 {
            r: 1,
            g: 2,
            b: 3,
            a: 128
        }
    );
    assert_eq!(
        parse_color("rgba( 1 , 2 , 3 , 1 )"),
        Rgba8 {
            r: 1,
            g: 2,
            b: 3,
            a: 255
        }
    );
    assert_eq!(
        parse_color("rgb(0, 128, 255)"),
        Rgba8 {
            r: 0,
            g: 128,
            b: 255,
            a: 255
        }
    );
    assert_eq!(
        parse_color("red"),
        Rgba8 {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        }
    );
    assert_eq!(
        parse_color("RED"),
        Rgba8 {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        }
    );
    assert_eq!(
        parse_color("transparent"),
        Rgba8 {
            r: 0,
            g: 0,
            b: 0,
            a: 0
        }
    );
    let black = Rgba8 {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    assert_eq!(parse_color(""), black);
    assert_eq!(parse_color("   "), black);
    assert_eq!(parse_color("not-a-color"), black);
    assert_eq!(parse_color("#12345"), black);
    assert_eq!(parse_color("#zzzzzz"), black);
    assert_eq!(parse_color("rgba(1,2)"), black);
    assert_eq!(parse_color("rgb(1,2,3"), black);
}

#[test]
fn timing_jitter_clamps_to_spec_band() {
    assert_eq!(bench_jitter(0.0), 0.015);
    assert_eq!(bench_jitter(100.0), 0.05);
    assert_eq!(bench_jitter(-100.0), -0.02);
    for g in [-5.0f64, -1.0, 0.0, 1.0, 5.0] {
        assert!((-0.02..=0.05).contains(&bench_jitter(g)));
    }
    assert!((bench::bench_scale(2.0, 0.0) - 2.0).abs() < 1e-9);
    assert!(bench::bench_scale(1.0, 0.05) <= 1.05);
    assert!((bench::bench_scale(1.0, bench::bench_jitter(0.0)) - 1.015).abs() < 1e-9);
    assert!(bench::bench_scale(1.0, -0.02) >= 0.98);
    assert!(bench::bench_scale(f64::NAN, 0.0) == 1.0);
    assert_eq!(bench::bench_scale(2.0, 0.0), bench::bench_scale(2.0, 0.0));
}

#[test]
fn canvas_cost_scales_with_cpu_and_stays_positive() {
    assert_eq!(CANVAS_OP_FILL_RECT, 0);
    assert_eq!(CANVAS_OP_FILL_TEXT, 1);
    assert_eq!(CANVAS_OP_GET_IMAGE_DATA, 2);
    assert_eq!(CANVAS_OP_TO_URL, 3);
    assert_eq!(CANVAS_OP_MEASURE, 4);
    let ops = [
        CANVAS_OP_FILL_RECT,
        CANVAS_OP_FILL_TEXT,
        CANVAS_OP_GET_IMAGE_DATA,
        CANVAS_OP_TO_URL,
        CANVAS_OP_MEASURE,
    ];
    for op in ops {
        let slow = canvas_time_cost_us(op, 0.5, 0.0);
        let base = canvas_time_cost_us(op, 1.0, 0.0);
        let fast = canvas_time_cost_us(op, 2.0, 0.0);
        assert!(slow >= 1 && base >= 1 && fast >= 1);
        assert!(slow <= base && base <= fast);
        assert!(canvas_time_cost_us(op, 1.0, -100.0) <= canvas_time_cost_us(op, 1.0, 100.0));
    }
    let to_url = canvas_time_cost_us(CANVAS_OP_TO_URL, 1.0, 0.0);
    assert_eq!(to_url, 55);
    assert!(canvas_time_cost_us(CANVAS_OP_FILL_RECT, 1.0, 0.0) < to_url);
    assert_eq!(
        canvas_time_cost_us(200, 1.0, 0.3),
        canvas_time_cost_us(CANVAS_OP_FILL_RECT, 1.0, 0.3)
    );
}

#[test]
fn audio_fp_lands_in_chrome_band() {
    let v = payload_gen::audio_fp(42);
    assert!((124.0..124.1).contains(&v), "audio fp out of band: {v}");
    assert_eq!(v, payload_gen::audio_fp(42));
}
