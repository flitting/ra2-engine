//! The app's icon, drawn here: a gold star on a red radar screen. (Not the
//! game's own icon, which is one of the game's files and so not ours to hand
//! out.)

use image::{Rgba, RgbaImage};

/// The icon `size` pixels square.
pub fn draw(size: u32) -> RgbaImage {
    // Each pixel is the average of a grid of samples across it.
    const SAMPLES: u32 = 4;
    let mut image = RgbaImage::new(size, size);
    for y in 0..size {
        for x in 0..size {
            let mut sum = [0.0f32; 4];
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let u = (x as f32 + (sx as f32 + 0.5) / SAMPLES as f32) / size as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / SAMPLES as f32) / size as f32;
                    let [r, g, b, a] = sample(u, v);
                    // Summed with the alpha already applied, so edges blend.
                    sum[0] += r * a;
                    sum[1] += g * a;
                    sum[2] += b * a;
                    sum[3] += a;
                }
            }
            let count = (SAMPLES * SAMPLES) as f32;
            let alpha = sum[3] / count;
            let channel = |c: f32| if alpha > 0.0 { (c / count / alpha * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
            image.put_pixel(x, y, Rgba([channel(sum[0]), channel(sum[1]), channel(sum[2]), (alpha * 255.0).round() as u8]));
        }
    }
    image
}

/// The colour at a point of the icon, `u` and `v` from 0 to 1 across and
/// down.
fn sample(u: f32, v: f32) -> [f32; 4] {
    // macOS's icon grid: the tile is 824 of 1024 across, its corners rounded
    // by 185.
    let (half, radius) = (412.0 / 1024.0, 185.0 / 1024.0);
    let (dx, dy) = ((u - 0.5).abs() - (half - radius), (v - 0.5).abs() - (half - radius));
    let outside = dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - radius;
    if outside > 0.0 {
        return [0.0; 4];
    }
    // A thin darker rim inside the edge.
    let rim = outside > -0.012;
    let shade = 1.0 - 0.35 * (v - 0.09) / 0.82;
    let mut colour = if rim { [0.45, 0.03, 0.03] } else { [0.86 * shade + 0.08, 0.10 * shade, 0.08 * shade] };
    // The radar's rings and cross-hairs, darker than the tile.
    let (cx, cy) = (u - 0.5, v - 0.5);
    let distance = cx.hypot(cy);
    let ring = [0.17, 0.27, 0.37].iter().any(|r| (distance - r).abs() < 0.006);
    let hair = (cx.abs() < 0.004 || cy.abs() < 0.004) && distance < 0.37;
    if !rim && (ring || hair) {
        colour = [colour[0] * 0.62, colour[1] * 0.62, colour[2] * 0.62];
    }
    // The star in the middle.
    let (sx, sy) = (cx / 0.26, (cy - 0.02) / 0.26);
    if in_star(sx, sy) {
        let glint = 1.0 - 0.18 * (sy + 1.0) / 2.0;
        colour = [1.0 * glint, 0.80 * glint, 0.22 * glint];
    }
    [colour[0], colour[1], colour[2], 1.0]
}

/// Whether a point is inside a five-pointed star of radius 1 about the origin,
/// point up.
fn in_star(x: f32, y: f32) -> bool {
    static CORNERS: std::sync::LazyLock<[(f32, f32); 10]> = std::sync::LazyLock::new(|| {
        std::array::from_fn(|i| {
            let angle = std::f32::consts::PI * (i as f32 / 5.0) - std::f32::consts::FRAC_PI_2;
            let r = if i % 2 == 0 { 1.0 } else { 0.382 };
            (r * angle.cos(), r * angle.sin())
        })
    });
    let corners = &*CORNERS;
    // Even-odd crossing count.
    let mut inside = false;
    for i in 0..corners.len() {
        let (a, b) = (corners[i], corners[(i + 1) % corners.len()]);
        if (a.1 > y) != (b.1 > y) && x < a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0) {
            inside = !inside;
        }
    }
    inside
}
