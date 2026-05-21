# `std.cv`

OpenCV-lite image loading, saving, and pixel transforms.

The CV stdlib exports `Size`, `Pixel`, and `Image` aliases for its
normalized sRGB image shape:

```text
Size = (Int,Int)
Pixel = (Real,(Real,Real))
Image = (Size,Seq[Seq[Pixel]])
```

`Size` is `(width,height)`. The outer sequence is image rows,
each inner sequence is a scanline, and pixels are sRGB triples written as
`(red,(green,blue))`. Channel values are normalized `Real` values in
`0.0..1.0`; codec nodes convert to and from 8-bit file samples at the
native boundary.

The row-matrix shape makes image geometry explicit. For numeric matrix
pipelines, use the channel matrix views, which return `Seq[Seq[Real]]`
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

save_jpeg   : (Bytes,Image) -> Faultable[Int]
save_png    : (Bytes,Image) -> Faultable[Int]
save_bmp    : (Bytes,Image) -> Faultable[Int]
save_pgm    : (Bytes,Image) -> Faultable[Int]
save_ppm    : (Bytes,Image) -> Faultable[Int]

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
width       : Image -> Int
height      : Image -> Int
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
threshold     : (Image,Real) -> Image
brighten      : (Image,Real) -> Image
darken        : (Image,Real) -> Image
contrast      : (Image,Real) -> Image
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
red_matrix   : Image -> Seq[Seq[Real]]
green_matrix : Image -> Seq[Seq[Real]]
blue_matrix  : Image -> Seq[Seq[Real]]
luma_matrix  : Image -> Seq[Seq[Real]]
```

These views bridge `std.cv` into `std.matrix`:

```flow
import std.cli { Args }
import std.cv { Image, luma_matrix, load }
import std.matrix { mean }

node average_luma(image: Image) -> value: Real {
    $image -> luma_matrix -> mean -> $value
}

program main(args: Args) -> exit_code: Faultable[Int] {
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

program main(args: Args) -> exit_code: Faultable[Int] {
    "input.png" -> load -> make_gray -> $image
    ("copy.jpg", $image) -> save_jpeg -> $exit_code
}
```
