mod data;

pub const WIDTH: usize = data::WIDTH;
pub const LINE_HEIGHT: usize = 17;

pub struct RasterizedChar {
    raster: &'static [[u8; WIDTH]; data::HEIGHT],
}

impl RasterizedChar {
    pub const fn raster(&self) -> &'static [[u8; WIDTH]; data::HEIGHT] {
        self.raster
    }
}

const fn half_block(upper: bool) -> [[u8; WIDTH]; data::HEIGHT] {
    let mut raster = [[0; WIDTH]; data::HEIGHT];
    let mut row = 0;
    while row < data::HEIGHT {
        if upper == (row < data::HEIGHT / 2) {
            raster[row] = [255; WIDTH];
        }
        row += 1;
    }
    raster
}

static UPPER_BLOCK: [[u8; WIDTH]; data::HEIGHT] = half_block(true);
static LOWER_BLOCK: [[u8; WIDTH]; data::HEIGHT] = half_block(false);
static FULL_BLOCK: [[u8; WIDTH]; data::HEIGHT] = [[255; WIDTH]; data::HEIGHT];

pub fn glyph(codepoint: u32) -> Option<RasterizedChar> {
    let raster = match codepoint {
        0x2580 => &UPPER_BLOCK,
        0x2584 => &LOWER_BLOCK,
        0x2588 => &FULL_BLOCK,
        _ => {
            let index = if (0x20..=0x7e).contains(&codepoint) {
                (codepoint - 0x20) as usize
            } else {
                (b'?' - 0x20) as usize
            };
            &data::GLYPHS[index]
        }
    };
    Some(RasterizedChar { raster })
}
