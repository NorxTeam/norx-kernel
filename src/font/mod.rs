mod data;

pub const WIDTH: usize = data::WIDTH;
pub const LINE_HEIGHT: usize = 18;

pub struct RasterizedChar {
    raster: &'static [[u8; WIDTH]; data::HEIGHT],
}

impl RasterizedChar {
    pub const fn raster(&self) -> &'static [[u8; WIDTH]; data::HEIGHT] {
        self.raster
    }
}

pub fn glyph(byte: u8) -> Option<RasterizedChar> {
    let index = if (0x20..=0x7e).contains(&byte) {
        (byte - 0x20) as usize
    } else {
        (b'?' - 0x20) as usize
    };
    Some(RasterizedChar {
        raster: &data::GLYPHS[index],
    })
}
