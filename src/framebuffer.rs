use crate::{
    boot::{PixelFormat, RawFramebuffer},
    font,
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
        self.raw.stride as u64
    }

    pub fn clear(&mut self, rgb: u32) {
        for y in 0..self.raw.height as u64 {
            for x in 0..self.raw.width as u64 {
                self.put_pixel(x, y, rgb);
            }
        }
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

    pub fn scroll_text(&mut self, top_row: usize, bottom_row: usize, bg: u32) {
        if bottom_row <= top_row + 1 {
            return;
        }

        let row_bytes = self.raw.stride.saturating_mul(TERM_H);
        let source = top_row.saturating_add(1).saturating_mul(row_bytes);
        let destination = top_row.saturating_mul(row_bytes);
        let count = bottom_row
            .saturating_sub(top_row + 1)
            .saturating_mul(row_bytes);

        if source.saturating_add(count) > self.raw.size
            || destination.saturating_add(count) > self.raw.size
        {
            return;
        }

        unsafe {
            core::ptr::copy(
                self.raw.base.add(source),
                self.raw.base.add(destination),
                count,
            );
        }
        self.rect(
            0,
            bottom_row.saturating_sub(1).saturating_mul(TERM_H) as u64,
            self.raw.width as u64,
            TERM_H as u64,
            bg,
        );
    }

    fn rect(&mut self, x: u64, y: u64, w: u64, h: u64, rgb: u32) {
        for yy in y..y.saturating_add(h) {
            for xx in x..x.saturating_add(w) {
                self.put_pixel(xx, yy, rgb);
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

    fn put_pixel(&mut self, x: u64, y: u64, rgb: u32) {
        if x >= self.raw.width as u64 || y >= self.raw.height as u64 {
            return;
        }

        let offset = y as usize * self.raw.stride + x as usize * self.raw.bytes_per_pixel;
        if offset + self.raw.bytes_per_pixel > self.raw.size {
            return;
        }

        let bytes = unsafe { core::slice::from_raw_parts_mut(self.raw.base, self.raw.size) };
        match self.raw.format {
            PixelFormat::Rgb => {
                bytes[offset] = (rgb >> 16) as u8;
                bytes[offset + 1] = (rgb >> 8) as u8;
                bytes[offset + 2] = rgb as u8;
                if self.raw.bytes_per_pixel == 4 {
                    bytes[offset + 3] = 0xff;
                }
            }
            PixelFormat::Bgr => {
                bytes[offset] = rgb as u8;
                bytes[offset + 1] = (rgb >> 8) as u8;
                bytes[offset + 2] = (rgb >> 16) as u8;
                if self.raw.bytes_per_pixel == 4 {
                    bytes[offset + 3] = 0xff;
                }
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
