mod data;

pub const WIDTH: usize = data::WIDTH;
pub const LINE_HEIGHT: usize = data::HEIGHT;
pub const SCALE: usize = 2;

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

const fn box_line(left: bool, right: bool, top: bool, bottom: bool) -> [[u8; WIDTH]; data::HEIGHT] {
    let mut raster = [[0; WIDTH]; data::HEIGHT];
    let mut row = 0;
    while row < data::HEIGHT {
        let horizontal = row == data::HEIGHT / 2 - 1 || row == data::HEIGHT / 2;
        let vertical = (top && row < data::HEIGHT / 2)
            || (bottom && row >= data::HEIGHT / 2)
            || (top || bottom) && (row == data::HEIGHT / 2 - 1 || row == data::HEIGHT / 2);
        let mut column = 0;
        while column < WIDTH {
            let horizontal_arm =
                horizontal && ((left && column < WIDTH / 2) || (right && column >= WIDTH / 2));
            let vertical_arm = vertical && (column == WIDTH / 2 - 1 || column == WIDTH / 2);
            if horizontal_arm || vertical_arm {
                raster[row][column] = 255;
            }
            column += 1;
        }
        row += 1;
    }
    raster
}

const fn outline_square() -> [[u8; WIDTH]; data::HEIGHT] {
    let mut raster = [[0; WIDTH]; data::HEIGHT];
    let mut row = 0;
    while row < data::HEIGHT {
        let mut column = 0;
        while column < WIDTH {
            if row == 1 || row == data::HEIGHT - 2 || column == 1 || column == WIDTH - 2 {
                raster[row][column] = 255;
            }
            column += 1;
        }
        row += 1;
    }
    raster
}

static HORIZONTAL_LINE: [[u8; WIDTH]; data::HEIGHT] = box_line(true, true, false, false);
static VERTICAL_LINE: [[u8; WIDTH]; data::HEIGHT] = box_line(false, false, true, true);
static TOP_LEFT: [[u8; WIDTH]; data::HEIGHT] = box_line(false, true, false, true);
static TOP_RIGHT: [[u8; WIDTH]; data::HEIGHT] = box_line(true, false, false, true);
static BOTTOM_LEFT: [[u8; WIDTH]; data::HEIGHT] = box_line(false, true, true, false);
static BOTTOM_RIGHT: [[u8; WIDTH]; data::HEIGHT] = box_line(true, false, true, false);
static WHITE_SQUARE: [[u8; WIDTH]; data::HEIGHT] = outline_square();

pub fn glyph(codepoint: u32) -> Option<RasterizedChar> {
    let raster = match codepoint {
        0x2500 => &HORIZONTAL_LINE,
        0x2502 => &VERTICAL_LINE,
        0x250c => &TOP_LEFT,
        0x2510 => &TOP_RIGHT,
        0x2514 => &BOTTOM_LEFT,
        0x2518 => &BOTTOM_RIGHT,
        0x25a0 => &FULL_BLOCK,
        0x25a1 => &WHITE_SQUARE,
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
