# `std.cv`

OpenCV-lite image loading, saving, and pixel transforms.

The CV stdlib exports `Size`, `Pixel`, and `Image` aliases for its
normalized sRGB image shape:

```text
Size = (i64,i64)
Pixel = (f64,(f64,f64))
Image = (Size,Seq[Seq[Pixel]])
```

`Size` is `(width,height)`. The outer sequence is image rows,
each inner sequence is a scanline, and pixels are sRGB triples written as
`(red,(green,blue))`. Channel values are normalized `f64` values in
`0.0..1.0`; codec nodes convert to and from 8-bit file samples at the
native boundary.

The row-matrix shape makes image geometry explicit. For numeric matrix
pipelines, use the channel matrix views, which return `Seq[Seq[f64]]`
and can be passed directly to `std.matrix`.

## Codecs

```text
load        : Bytes -> Faultable[Image]
load_jpeg   : Bytes -> Faultable[Image]
load_png    : Bytes -> Faultable[Image]
load_bmp    : Bytes -> Faultable[Image]
load_pnm    : Bytes -> Faultable[Image]
load_pgm    : Bytes -> Faultable[Image]
load_ppm    : Bytes -> Faultable[Image]

decode      : Bytes -> Faultable[Image]
decode_jpeg : Bytes -> Faultable[Image]
decode_png  : Bytes -> Faultable[Image]
decode_bmp  : Bytes -> Faultable[Image]
decode_pnm  : Bytes -> Faultable[Image]
decode_pgm  : Bytes -> Faultable[Image]
decode_ppm  : Bytes -> Faultable[Image]

save_jpeg   : (Bytes,Image) -> Faultable[i64]
save_png    : (Bytes,Image) -> Faultable[i64]
save_bmp    : (Bytes,Image) -> Faultable[i64]
save_pgm    : (Bytes,Image) -> Faultable[i64]
save_ppm    : (Bytes,Image) -> Faultable[i64]

encode_jpeg : Image -> Faultable[Bytes]
encode_png  : Image -> Faultable[Bytes]
encode_bmp  : Image -> Faultable[Bytes]
encode_pgm  : Image -> Faultable[Bytes]
encode_ppm  : Image -> Faultable[Bytes]
```

`load` and `decode` detect JPEG, PNG, BMP, PGM, and PPM from magic bytes.
`save` is intentionally not exported; choose an explicit output codec.

## Image Access

```text
dimensions  : Image -> Size
width       : Image -> i64
height      : Image -> i64
pixel_rows  : Image -> Seq[Seq[Pixel]]
pixels      : Image -> Seq[Pixel]
map_pixels  : (Image,Seq[Seq[Pixel]]) -> Image
normalize   : Bytes -> Image
```

`pixels` flattens the row matrix in row-major order. `normalize` treats
raw bytes as a one-row grayscale image and is mainly useful for small
byte-to-image adapters and tests.

## Transforms

```text
grayscale     : Image -> Image
invert        : Image -> Image
threshold     : (Image,f64) -> Image
brighten      : (Image,f64) -> Image
darken        : (Image,f64) -> Image
contrast      : (Image,f64) -> Image
red_channel   : Image -> Image
green_channel : Image -> Image
blue_channel  : Image -> Image
sepia         : Image -> Image
add           : (Image,Image) -> Image
sub           : (Image,Image) -> Image
absdiff       : (Image,Image) -> Image
```

Channel values are clamped to `0.0..1.0` where transforms can overflow.
`contrast` uses `1.0` as neutral scale.

## Matrix Views

```text
red_matrix   : Image -> Seq[Seq[f64]]
green_matrix : Image -> Seq[Seq[f64]]
blue_matrix  : Image -> Seq[Seq[f64]]
luma_matrix  : Image -> Seq[Seq[f64]]
```

These views bridge `std.cv` into `std.matrix`:

```flow
import std.cli { Args }
import std.cv { Image, luma_matrix, load }
import std.matrix { mean }

node average_luma(image: Image) -> value: f64 {
    $image -> luma_matrix -> mean -> $value
}

program main(args: Args) -> exit_code: Faultable[i64] {
    "input.png" -> load -> average_luma -> $value
    0 -> $exit_code
}
```

## Example

```flow
import std.cli { Args }
import std.cv { Image, grayscale, load, save_jpeg }

node make_gray(image: Image) -> out: Image {
    $image -> grayscale -> $out
}

program main(args: Args) -> exit_code: Faultable[i64] {
    "input.png" -> load -> make_gray -> $image
    ("copy.jpg", $image) -> save_jpeg -> $exit_code
}
```
