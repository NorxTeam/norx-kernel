use noto_sans_mono_bitmap::{get_raster, get_raster_width, FontWeight, RasterHeight};

const WEIGHT: FontWeight = FontWeight::Regular;
const HEIGHT_KIND: RasterHeight = RasterHeight::Size20;

pub const WIDTH: usize = get_raster_width(WEIGHT, HEIGHT_KIND);
pub const LINE_HEIGHT: usize = 22;

pub fn glyph(byte: u8) -> Option<noto_sans_mono_bitmap::RasterizedChar> {
    let ch = if byte.is_ascii() { byte as char } else { '?' };
    get_raster(ch, WEIGHT, HEIGHT_KIND).or_else(|| get_raster('?', WEIGHT, HEIGHT_KIND))
}
