# grayscale-image

Reads an input JPEG filepath and an output JPEG filepath from positional
command-line arguments, converts the decoded image to grayscale, and saves the
result.

```text
$ flowarrow run main.flow input.jpg output.jpg
```

## Why this example matters

It shows the source-backed CV pipeline end to end:

1. `argv` provides the two file paths as dataflow values.
2. `load_jpeg` reads and decodes the input into the standard `std.cv`
   RGB image format.
3. `grayscale` operates on that normalized image format.
4. `save_jpeg` encodes the grayscale image and writes it to the output path.

The current `std.cv` JPEG boundary uses libjpeg/jpeg-turbo at runtime.
