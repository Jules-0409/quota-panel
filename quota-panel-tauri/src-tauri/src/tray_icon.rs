//! 菜单栏托盘图标。
//!
//! macOS 的菜单栏图标该是「单色模板图」：系统只取 alpha 通道，按菜单栏的明暗自动上色，
//! 这样浅色/深色菜单栏都不用改资源。彩色 App 图标缩到 18pt 只能糊成一团色块。
//!
//! 这里画到内存里，不走 PNG：一来不想为了读一张小图给 tauri 打开 `image-png`
//! feature（会拖进整个 image crate），二来画出来的形状能被单元测试钉住。
//! 形状沿用 App 图标的隐喻——圆环 + 中心点，缺口朝左上。

/// 图标边长（像素）。tray-icon 会把菜单栏图标的高度定成 18pt，36 = 18pt @2x。
pub const SIZE: u32 = 36;

/// 每像素的超采样倍率，靠它得到抗锯齿的 alpha。
const SUPERSAMPLE: u32 = 4;

/// 描边粗细（像素），约 1.5pt，跟系统菜单栏图标的笔画差不多。
const STROKE: f32 = 3.0;

/// 描边中心线到中心的半径：留 2px 外边距，再把描边的一半收进来。
const RING_RADIUS: f32 = SIZE as f32 / 2.0 - 2.0 - STROKE / 2.0;

/// 中心点的半径。
const DOT_RADIUS: f32 = 2.4;

/// 缺口的角度范围（度）。图像坐标 y 向下，所以 -90° 是正上方，-165°..-95° 即左上方。
const GAP: (f32, f32) = (-165.0, -95.0);

/// 画一张 RGBA 托盘图标：RGB 全黑（模板图只看 alpha），形状是「缺口朝左上的圆环 + 中心点」。
pub fn template_rgba() -> Vec<u8> {
    let center = (SIZE as f32 - 1.0) / 2.0;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    let total = SUPERSAMPLE * SUPERSAMPLE;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let mut hits = 0u32;
            for sy in 0..SUPERSAMPLE {
                for sx in 0..SUPERSAMPLE {
                    // 在像素内部再取 4x4 个采样点，数命中比例当 alpha
                    let px = x as f32 + (sx as f32 + 0.5) / SUPERSAMPLE as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / SUPERSAMPLE as f32;
                    let (dx, dy) = (px - center, py - center);
                    let distance = (dx * dx + dy * dy).sqrt();
                    let angle = dy.atan2(dx).to_degrees();
                    let on_ring = (distance - RING_RADIUS).abs() <= STROKE / 2.0
                        && !(angle > GAP.0 && angle < GAP.1);
                    if on_ring || distance <= DOT_RADIUS {
                        hits += 1;
                    }
                }
            }
            rgba[((y * SIZE + x) * 4 + 3) as usize] = (hits * 255 / total) as u8;
        }
    }

    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_at(x: u32, y: u32) -> u8 {
        template_rgba()[((y * SIZE + x) * 4 + 3) as usize]
    }

    /// 环上某个角度、半径落在描边中心线上的像素坐标
    fn on_ring(angle_degrees: f32) -> (u32, u32) {
        let center = (SIZE as f32 - 1.0) / 2.0;
        let radians = angle_degrees.to_radians();
        let x = center + RING_RADIUS * radians.cos();
        let y = center + RING_RADIUS * radians.sin();
        (x.round() as u32, y.round() as u32)
    }

    #[test]
    fn buffer_is_rgba_and_only_uses_alpha() {
        let rgba = template_rgba();
        assert_eq!(rgba.len(), (SIZE * SIZE * 4) as usize);
        assert!(
            rgba.chunks(4).all(|p| p[..3] == [0, 0, 0]),
            "模板图只该有 alpha，RGB 必须全黑，否则系统上色会跑偏"
        );
    }

    #[test]
    fn center_dot_is_solid() {
        assert_eq!(alpha_at(SIZE / 2, SIZE / 2), 255);
    }

    #[test]
    fn ring_is_drawn_everywhere_except_the_gap() {
        for angle in [-90.0, -45.0, 0.0, 45.0, 90.0, 135.0, 180.0] {
            let (x, y) = on_ring(angle);
            // 取整会落在描边边沿上，所以只要求「大部分被盖住」，不要求实心
            assert!(alpha_at(x, y) > 128, "角度 {angle} 的环上没有被盖住");
        }
        // 缺口正中（-130°）：环上这里必须是空的，否则缺口就白留了
        let (x, y) = on_ring(-130.0);
        assert_eq!(alpha_at(x, y), 0, "缺口没让开");
    }

    #[test]
    fn icon_keeps_a_transparent_margin() {
        for i in 0..SIZE {
            for (x, y) in [(i, 0), (i, SIZE - 1), (0, i), (SIZE - 1, i)] {
                assert_eq!(alpha_at(x, y), 0, "({x},{y}) 压到画布边缘了");
            }
        }
    }
}
