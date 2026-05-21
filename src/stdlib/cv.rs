use super::*;

const MODULE: &str = "std.cv.native";
const IMAGE: &str = "((Int,Int),Seq[Seq[(Real,(Real,Real))]])";
const CV_TYPES_H: &str = include_str!("cv.h");
const CV_NATIVE_H: &str = include_str!("cv_native.h");
pub const TYPE_H: &[&str] = &[CV_TYPES_H];
pub const H: &[&str] = &[CV_NATIVE_H];
pub const C: &str = include_str!("cv.c");

pub const DECODE: StdSymbol = node(
    MODULE,
    "decode",
    "Bytes",
    "Faultable[((Int,Int),Seq[Seq[(Real,(Real,Real))]])]",
);
pub const DECODE_BMP: StdSymbol = node(
    MODULE,
    "decode_bmp",
    "Bytes",
    "Faultable[((Int,Int),Seq[Seq[(Real,(Real,Real))]])]",
);
pub const DECODE_JPEG: StdSymbol = node(
    MODULE,
    "decode_jpeg",
    "Bytes",
    "Faultable[((Int,Int),Seq[Seq[(Real,(Real,Real))]])]",
);
pub const DECODE_PNG: StdSymbol = node(
    MODULE,
    "decode_png",
    "Bytes",
    "Faultable[((Int,Int),Seq[Seq[(Real,(Real,Real))]])]",
);
pub const DECODE_PNM: StdSymbol = node(
    MODULE,
    "decode_pnm",
    "Bytes",
    "Faultable[((Int,Int),Seq[Seq[(Real,(Real,Real))]])]",
);
pub const ENCODE_BMP: StdSymbol = node(MODULE, "encode_bmp", IMAGE, "Faultable[Bytes]");
pub const ENCODE_JPEG: StdSymbol = node(MODULE, "encode_jpeg", IMAGE, "Faultable[Bytes]");
pub const ENCODE_PGM: StdSymbol = node(MODULE, "encode_pgm", IMAGE, "Faultable[Bytes]");
pub const ENCODE_PNG: StdSymbol = node(MODULE, "encode_png", IMAGE, "Faultable[Bytes]");
pub const ENCODE_PPM: StdSymbol = node(MODULE, "encode_ppm", IMAGE, "Faultable[Bytes]");
