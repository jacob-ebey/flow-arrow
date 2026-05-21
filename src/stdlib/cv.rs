use super::*;

const MODULE: &str = "std.cv.native";
const IMAGE: &str = "((Int,Int),Seq[(Int,(Int,Int))])";
pub const C: &str = include_str!("cv.c");

pub const DECODE_JPEG: StdSymbol = node(
    MODULE,
    "decode_jpeg",
    "Bytes",
    "Faultable[((Int,Int),Seq[(Int,(Int,Int))])]",
);
pub const ENCODE_JPEG: StdSymbol = node(MODULE, "encode_jpeg", IMAGE, "Faultable[Bytes]");
