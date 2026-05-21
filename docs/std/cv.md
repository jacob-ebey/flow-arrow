# `std.cv`

JPEG-backed image helpers.

The CV stdlib uses a standard RGB image shape:

```text
Image = ((Int,Int),Seq[(Int,(Int,Int))])
```

The first tuple is `(width,height)`. Pixels are row-major RGB triples,
written as `(red,(green,blue))` so they can be addressed with the
two-element tuple helpers available today.

JPEG helpers use libjpeg/jpeg-turbo to decode and encode real JPEG image
data. PNG helpers are reserved but not implemented by this backend yet.

## Nodes

```text
normalize   : Bytes         -> Image
grayscale   : Image         -> Image
load        : Bytes         -> Faultable[Image]
save        : (Bytes,Image) -> Faultable[Int]
load_jpeg   : Bytes         -> Faultable[Image]
save_jpeg   : (Bytes,Image) -> Faultable[Int]
decode_jpeg : Bytes         -> Faultable[Image]
encode_jpeg : Image         -> Faultable[Bytes]
```

`save` and `save_jpeg` take `(path, image)`.

## Semantics

- `normalize` converts bytes to `Image` using `width = byte_length`,
  `height = 1`, and grayscale RGB pixels.
- `grayscale` maps every RGB pixel to `(average, (average, average))`.
- `load` and `load_jpeg` read a JPEG file from disk and decode it to
  `Image`.
- `save` and `save_jpeg` encode an `Image` as JPEG and write it to disk.
- `decode_jpeg` decodes encoded JPEG bytes to `Image`.
- `encode_jpeg` encodes an `Image` to JPEG bytes.
- PNG helpers are not part of the current public `std.cv` surface.

## Example

```flow
import std.cli { Args }
import std.cv { grayscale, load_jpeg, save_jpeg }

program main(args: Args) -> exit_code: Faultable[Int] {
    "input.jpg" -> load_jpeg -> grayscale -> $image
    ("copy.jpg", $image) -> save_jpeg -> $exit_code
}
```
