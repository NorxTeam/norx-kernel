use crate::{
    error::KernelError,
    font,
    uefi::{PixelFormat, RawFramebuffer},
};

pub const TERM_W: usize = font::WIDTH;
pub const TERM_H: usize = font::LINE_HEIGHT;

pub struct Fb {
    raw: RawFramebuffer,
}

pub fn init(raw: RawFramebuffer) -> Fb {
    Fb { raw }
}

impl Fb {
    pub fn width(&self) -> u64 {
        self.raw.width as u64
    }

    pub fn height(&self) -> u64 {
        self.raw.height as u64
    }

    pub fn pitch(&self) -> u64 {
        (self.raw.stride * 4) as u64
    }

    pub fn clear(&mut self, rgb: u32) {
        for y in 0..self.raw.height as u64 {
            for x in 0..self.raw.width as u64 {
                self.put_pixel(x, y, rgb);
            }
        }
    }

    pub fn crash(&mut self, error: KernelError) {
        self.clear(0x16191f);
        let cx = self.raw.width as u64 / 2;
        let cy = self.raw.height as u64 / 2;

        self.ring(cx, cy.saturating_sub(148), 52, 0xe8edf4);
        self.rect(
            cx.saturating_sub(4),
            cy.saturating_sub(178),
            8,
            42,
            0xe8edf4,
        );
        self.rect(cx.saturating_sub(4), cy.saturating_sub(124), 8, 8, 0xe8edf4);

        self.text_centered(cy.saturating_sub(54), error.title(), 3, 0xf4f7fb);
        self.text_centered(cy + 4, error.message, 2, 0xc8d0da);
        self.text_centered(cy + 42, error.detail, 1, 0xaeb7c2);
        self.text_centered(cy + 76, "CODE", 1, 0x7f8894);
        self.hex_centered(cy + 98, error.code, 1, 0xaeb7c2);
        self.text_centered(cy + 132, "ARG0", 1, 0x7f8894);
        self.hex_centered(cy + 154, error.arg0, 1, 0xaeb7c2);
        self.text_centered(cy + 188, "ARG1", 1, 0x7f8894);
        self.hex_centered(cy + 210, error.arg1, 1, 0xaeb7c2);
        self.text_centered(cy + 250, "SYSTEM HALTED", 1, 0x7f8894);
    }

    pub fn term_char(&mut self, x: usize, y: usize, byte: u8, fg: u32, bg: u32) {
        let x = x as u64;
        let y = y as u64;
        self.rect(x, y, TERM_W as u64, TERM_H as u64, bg);
        self.terminal_glyph(x, y, byte, fg);
    }

    pub fn term_cursor(&mut self, x: usize, y: usize, on: bool, fg: u32, bg: u32) {
        let color = if on { fg } else { bg };
        self.rect(
            x as u64,
            y as u64 + TERM_H as u64 - 2,
            TERM_W as u64,
            2,
            color,
        );
    }

    fn rect(&mut self, x: u64, y: u64, w: u64, h: u64, rgb: u32) {
        for yy in y..y.saturating_add(h) {
            for xx in x..x.saturating_add(w) {
                self.put_pixel(xx, yy, rgb);
            }
        }
    }

    fn ring(&mut self, cx: u64, cy: u64, r: u64, rgb: u32) {
        let inner = r.saturating_sub(4) * r.saturating_sub(4);
        let outer = r * r;
        for y in cy.saturating_sub(r)..cy.saturating_add(r) {
            for x in cx.saturating_sub(r)..cx.saturating_add(r) {
                let dx = x.abs_diff(cx);
                let dy = y.abs_diff(cy);
                let d = dx * dx + dy * dy;
                if d >= inner && d <= outer {
                    self.put_pixel(x, y, rgb);
                }
            }
        }
    }

    fn text_centered(&mut self, y: u64, text: &str, scale: u64, rgb: u32) {
        let width = text.len() as u64 * TERM_W as u64 * scale;
        let x = (self.raw.width as u64).saturating_sub(width) / 2;
        self.text(x, y, text, scale, rgb);
    }

    fn text(&mut self, x: u64, y: u64, text: &str, scale: u64, rgb: u32) {
        let mut cursor = x;
        for byte in text.bytes() {
            self.glyph(cursor, y, byte, scale, rgb);
            cursor += TERM_W as u64 * scale;
        }
    }

    fn glyph(&mut self, x: u64, y: u64, byte: u8, scale: u64, rgb: u32) {
        let Some(glyph) = font::glyph(byte) else {
            return;
        };
        for (yy, row) in glyph.raster().iter().enumerate() {
            for (xx, alpha) in row.iter().enumerate() {
                if *alpha != 0 {
                    self.rect(
                        x + xx as u64 * scale,
                        y + yy as u64 * scale,
                        scale,
                        scale,
                        scale_rgb(rgb, *alpha),
                    );
                }
            }
        }
    }

    fn terminal_glyph(&mut self, x: u64, y: u64, byte: u8, rgb: u32) {
        let Some(glyph) = font::glyph(byte) else {
            return;
        };
        for (yy, row) in glyph.raster().iter().enumerate() {
            for (xx, alpha) in row.iter().enumerate() {
                if *alpha != 0 {
                    self.put_pixel(x + xx as u64, y + yy as u64, scale_rgb(rgb, *alpha));
                }
            }
        }
    }

    fn hex_centered(&mut self, y: u64, value: u64, scale: u64, rgb: u32) {
        let chars = 18;
        let width = chars * TERM_W as u64 * scale;
        let x = (self.raw.width as u64).saturating_sub(width) / 2;
        self.text(x, y, "0x", scale, rgb);
        for i in 0..16 {
            let shift = (15 - i) * 4;
            let nibble = ((value >> shift) & 0xf) as u8;
            let ch = if nibble < 10 {
                b'0' + nibble
            } else {
                b'a' + nibble - 10
            };
            self.glyph(x + (2 + i) * TERM_W as u64 * scale, y, ch, scale, rgb);
        }
    }

    fn put_pixel(&mut self, x: u64, y: u64, rgb: u32) {
        if x >= self.raw.width as u64 || y >= self.raw.height as u64 {
            return;
        }

        let offset = (y as usize * self.raw.stride + x as usize) * 4;
        if offset + 3 >= self.raw.size {
            return;
        }

        let bytes = unsafe { core::slice::from_raw_parts_mut(self.raw.base, self.raw.size) };
        match self.raw.format {
            PixelFormat::Rgb => {
                bytes[offset] = (rgb >> 16) as u8;
                bytes[offset + 1] = (rgb >> 8) as u8;
                bytes[offset + 2] = rgb as u8;
            }
            PixelFormat::Bgr => {
                bytes[offset] = rgb as u8;
                bytes[offset + 1] = (rgb >> 8) as u8;
                bytes[offset + 2] = (rgb >> 16) as u8;
            }
        }
    }
}

fn scale_rgb(rgb: u32, alpha: u8) -> u32 {
    let alpha = alpha as u32;
    let r = ((rgb >> 16) & 0xff) * alpha / 255;
    let g = ((rgb >> 8) & 0xff) * alpha / 255;
    let b = (rgb & 0xff) * alpha / 255;
    (r << 16) | (g << 8) | b
}
