#include "cv.h"
#include "cv_native.h"

typedef struct {
  struct jpeg_error_mgr pub;
  jmp_buf setjmp_buffer;
  char message[JMSG_LENGTH_MAX];
} FaJpegError;

static void fa_cv_jpeg_error_exit(j_common_ptr cinfo) {
  FaJpegError *err = (FaJpegError *)cinfo->err;
  (*cinfo->err->format_message)(cinfo, err->message);
  longjmp(err->setjmp_buffer, 1);
}

static FaCvImageResult fa_cv_image_fault_cstr(const char *message) {
  FaCvImageResult out;
  out.is_fault = true;
  out.fault = fa_fault_cstr(message);
  return out;
}

static FaCvImageResult fa_cv_image_fault2(const char *op, const char *message) {
  char buffer[512];
  snprintf(buffer, sizeof(buffer), "%s: %s", op, message);
  return fa_cv_image_fault_cstr(buffer);
}

static FaCvImageResult fa_cv_image_ok(FaCvImage image) {
  FaCvImageResult out;
  out.is_fault = false;
  out.value = image;
  return out;
}

static FaCvImage fa_cv_image_new(size_t width, size_t height) {
  FaCvImage image;
  image.f0.f0 = (int64_t)width;
  image.f0.f1 = (int64_t)height;
  image.f1 = FaSeq_Seq_Tuple_Real_Tuple_Real_Real_new(height);
  for (size_t y = 0; y < height; y++) {
    image.f1.items[y] = FaSeq_Tuple_Real_Tuple_Real_Real_new(width);
  }
  return image;
}

static FaCvPixel *fa_cv_pixel_at(FaCvImage *image, size_t y, size_t x) {
  return &image->f1.items[y].items[x];
}

static bool fa_cv_channel_ok(double value) {
  return isfinite(value) && value >= 0.0 && value <= 1.0;
}

static double fa_cv_from_u8(unsigned char value) {
  return (double)value / 255.0;
}

static unsigned char fa_cv_to_u8(double value) {
  if (value <= 0.0) return 0;
  if (value >= 1.0) return 255;
  return (unsigned char)floor(value * 255.0 + 0.5);
}

static bool fa_cv_mul_overflows(size_t a, size_t b, size_t *out) {
  if (a != 0 && b > SIZE_MAX / a) return true;
  *out = a * b;
  return false;
}

static bool fa_cv_add_overflows(size_t a, size_t b, size_t *out) {
  if (b > SIZE_MAX - a) return true;
  *out = a + b;
  return false;
}

static bool fa_cv_prepare_size(
    const char *op,
    size_t width,
    size_t height,
    size_t *count,
    FaCvImageResult *fault
) {
  if (width == 0 || height == 0) {
    *fault = fa_cv_image_fault2(op, "image dimensions must be positive");
    return false;
  }
  if (width > (size_t)INT64_MAX || height > (size_t)INT64_MAX) {
    *fault = fa_cv_image_fault2(op, "image dimensions exceed Int limits");
    return false;
  }
  if (fa_cv_mul_overflows(width, height, count)) {
    *fault = fa_cv_image_fault2(op, "image is too large");
    return false;
  }
  if (*count > SIZE_MAX / sizeof(FaCvPixel)) {
    *fault = fa_cv_image_fault2(op, "image is too large");
    return false;
  }
  return true;
}

static bool fa_cv_prepare_image(
    FaCvImage image,
    const char *op,
    size_t *width,
    size_t *height,
    size_t *count,
    FaFault *fault
) {
  if (image.f0.f0 <= 0 || image.f0.f1 <= 0) {
    char message[128];
    snprintf(message, sizeof(message), "%s: image dimensions must be positive", op);
    *fault = fa_fault_cstr(message);
    return false;
  }
  *width = (size_t)image.f0.f0;
  *height = (size_t)image.f0.f1;
  if (fa_cv_mul_overflows(*width, *height, count)) {
    char message[128];
    snprintf(message, sizeof(message), "%s: image is too large", op);
    *fault = fa_fault_cstr(message);
    return false;
  }
  if (image.f1.count != *height) {
    char message[128];
    snprintf(message, sizeof(message), "%s: row count does not match height", op);
    *fault = fa_fault_cstr(message);
    return false;
  }
  for (size_t y = 0; y < *height; y++) {
    if (image.f1.items[y].count != *width) {
      char message[128];
      snprintf(message, sizeof(message), "%s: row width does not match image width", op);
      *fault = fa_fault_cstr(message);
      return false;
    }
    for (size_t x = 0; x < *width; x++) {
      FaCvPixel pixel = image.f1.items[y].items[x];
      double r = pixel.f0;
      double g = pixel.f1.f0;
      double b = pixel.f1.f1;
      if (!fa_cv_channel_ok(r) || !fa_cv_channel_ok(g) || !fa_cv_channel_ok(b)) {
        char message[128];
        snprintf(message, sizeof(message), "%s: sRGB channel outside 0.0..1.0", op);
        *fault = fa_fault_cstr(message);
        return false;
      }
    }
  }
  return true;
}

static unsigned char fa_cv_luma(FaCvPixel pixel) {
  double total = pixel.f0 * 0.299 + pixel.f1.f0 * 0.587 + pixel.f1.f1 * 0.114;
  return fa_cv_to_u8(total);
}

static FaCvImageResult fa_cv_decode_jpeg(FaBytes bytes) {
  struct jpeg_decompress_struct cinfo;
  FaJpegError jerr;
  memset(&cinfo, 0, sizeof(cinfo));
  cinfo.err = jpeg_std_error(&jerr.pub);
  jerr.pub.error_exit = fa_cv_jpeg_error_exit;
  if (setjmp(jerr.setjmp_buffer)) {
    char message[JMSG_LENGTH_MAX + 16];
    snprintf(message, sizeof(message), "decode_jpeg: %s", jerr.message);
    jpeg_destroy_decompress(&cinfo);
    return fa_cv_image_fault_cstr(message);
  }

  jpeg_create_decompress(&cinfo);
  jpeg_mem_src(&cinfo, (const unsigned char *)bytes.bytes, bytes.len);
  int header = jpeg_read_header(&cinfo, TRUE);
  if (header != JPEG_HEADER_OK) {
    jpeg_destroy_decompress(&cinfo);
    return fa_cv_image_fault_cstr("decode_jpeg: invalid JPEG header");
  }
  cinfo.out_color_space = JCS_RGB;
  jpeg_start_decompress(&cinfo);
  if (cinfo.output_components != 3) {
    jpeg_destroy_decompress(&cinfo);
    return fa_cv_image_fault_cstr("decode_jpeg: expected RGB output");
  }

  size_t width = (size_t)cinfo.output_width;
  size_t height = (size_t)cinfo.output_height;
  size_t count = 0;
  FaCvImageResult fault;
  if (!fa_cv_prepare_size("decode_jpeg", width, height, &count, &fault)) {
    jpeg_destroy_decompress(&cinfo);
    return fault;
  }

  FaCvImage image = fa_cv_image_new(width, height);

  size_t row_stride = width * 3;
  JSAMPARRAY row = (*cinfo.mem->alloc_sarray)((j_common_ptr)&cinfo, JPOOL_IMAGE, row_stride, 1);
  while (cinfo.output_scanline < cinfo.output_height) {
    JDIMENSION y = cinfo.output_scanline;
    jpeg_read_scanlines(&cinfo, row, 1);
    for (size_t x = 0; x < width; x++) {
      FaCvPixel *pixel = fa_cv_pixel_at(&image, (size_t)y, x);
      pixel->f0 = fa_cv_from_u8(row[0][x * 3]);
      pixel->f1.f0 = fa_cv_from_u8(row[0][x * 3 + 1]);
      pixel->f1.f1 = fa_cv_from_u8(row[0][x * 3 + 2]);
    }
  }
  jpeg_finish_decompress(&cinfo);
  jpeg_destroy_decompress(&cinfo);

  return fa_cv_image_ok(image);
}

static FaFaultable_Bytes fa_cv_encode_jpeg(FaCvImage image) {
  size_t width = 0;
  size_t height = 0;
  size_t count = 0;
  FaFault fault;
  if (!fa_cv_prepare_image(image, "encode_jpeg", &width, &height, &count, &fault)) {
    return FaFaultable_Bytes_fault(fault);
  }
  if (width > (size_t)((JDIMENSION)-1) || height > (size_t)((JDIMENSION)-1)) {
    return FaFaultable_Bytes_fault(fa_fault_cstr("encode_jpeg: dimensions exceed JPEG limits"));
  }

  struct jpeg_compress_struct cinfo;
  FaJpegError jerr;
  unsigned char *encoded = NULL;
  unsigned long encoded_len = 0;
  unsigned char *row = NULL;
  memset(&cinfo, 0, sizeof(cinfo));
  cinfo.err = jpeg_std_error(&jerr.pub);
  jerr.pub.error_exit = fa_cv_jpeg_error_exit;
  if (setjmp(jerr.setjmp_buffer)) {
    char message[JMSG_LENGTH_MAX + 16];
    snprintf(message, sizeof(message), "encode_jpeg: %s", jerr.message);
    jpeg_destroy_compress(&cinfo);
    free(row);
    free(encoded);
    return FaFaultable_Bytes_fault(fa_fault_cstr(message));
  }

  jpeg_create_compress(&cinfo);
  jpeg_mem_dest(&cinfo, &encoded, &encoded_len);
  cinfo.image_width = (JDIMENSION)width;
  cinfo.image_height = (JDIMENSION)height;
  cinfo.input_components = 3;
  cinfo.in_color_space = JCS_RGB;
  jpeg_set_defaults(&cinfo);
  jpeg_set_quality(&cinfo, 95, TRUE);
  jpeg_start_compress(&cinfo, TRUE);

  size_t row_stride = width * 3;
  row = (unsigned char *)malloc(row_stride);
  if (!row) fa_die_alloc();
  while (cinfo.next_scanline < cinfo.image_height) {
    size_t y = cinfo.next_scanline;
    for (size_t x = 0; x < width; x++) {
      FaCvPixel pixel = image.f1.items[y].items[x];
      row[x * 3] = fa_cv_to_u8(pixel.f0);
      row[x * 3 + 1] = fa_cv_to_u8(pixel.f1.f0);
      row[x * 3 + 2] = fa_cv_to_u8(pixel.f1.f1);
    }
    JSAMPROW row_pointer[1] = { row };
    jpeg_write_scanlines(&cinfo, row_pointer, 1);
  }
  jpeg_finish_compress(&cinfo);
  jpeg_destroy_compress(&cinfo);
  free(row);
  (void)count;

  return FaFaultable_Bytes_ok(fa_bytes_owned((char *)encoded, (size_t)encoded_len));
}

static FaCvImageResult fa_cv_decode_png(FaBytes bytes) {
  png_image png;
  memset(&png, 0, sizeof(png));
  png.version = PNG_IMAGE_VERSION;
  if (!png_image_begin_read_from_memory(&png, bytes.bytes, bytes.len)) {
    return fa_cv_image_fault2("decode_png", png.message);
  }
  png.format = PNG_FORMAT_RGBA;

  size_t width = (size_t)png.width;
  size_t height = (size_t)png.height;
  size_t count = 0;
  FaCvImageResult fault;
  if (!fa_cv_prepare_size("decode_png", width, height, &count, &fault)) {
    png_image_free(&png);
    return fault;
  }
  if (count > SIZE_MAX / 4) {
    png_image_free(&png);
    return fa_cv_image_fault_cstr("decode_png: image is too large");
  }

  size_t buffer_len = PNG_IMAGE_SIZE(png);
  unsigned char *buffer = (unsigned char *)malloc(buffer_len);
  if (!buffer) fa_die_alloc();
  if (!png_image_finish_read(&png, NULL, buffer, 0, NULL)) {
    FaCvImageResult out = fa_cv_image_fault2("decode_png", png.message);
    png_image_free(&png);
    free(buffer);
    return out;
  }

  FaCvImage image = fa_cv_image_new(width, height);
  for (size_t y = 0; y < height; y++) {
    for (size_t x = 0; x < width; x++) {
      size_t i = y * width + x;
      FaCvPixel *pixel = fa_cv_pixel_at(&image, y, x);
      pixel->f0 = fa_cv_from_u8(buffer[i * 4]);
      pixel->f1.f0 = fa_cv_from_u8(buffer[i * 4 + 1]);
      pixel->f1.f1 = fa_cv_from_u8(buffer[i * 4 + 2]);
    }
  }
  png_image_free(&png);
  free(buffer);
  return fa_cv_image_ok(image);
}

static FaFaultable_Bytes fa_cv_encode_png(FaCvImage image) {
  size_t width = 0;
  size_t height = 0;
  size_t count = 0;
  FaFault fault;
  if (!fa_cv_prepare_image(image, "encode_png", &width, &height, &count, &fault)) {
    return FaFaultable_Bytes_fault(fault);
  }
  if (width > PNG_UINT_31_MAX || height > PNG_UINT_31_MAX || count > SIZE_MAX / 3) {
    return FaFaultable_Bytes_fault(fa_fault_cstr("encode_png: image is too large"));
  }

  unsigned char *pixels = (unsigned char *)malloc(count * 3);
  if (!pixels) fa_die_alloc();
  for (size_t y = 0; y < height; y++) {
    for (size_t x = 0; x < width; x++) {
      size_t i = y * width + x;
      FaCvPixel pixel = image.f1.items[y].items[x];
      pixels[i * 3] = fa_cv_to_u8(pixel.f0);
      pixels[i * 3 + 1] = fa_cv_to_u8(pixel.f1.f0);
      pixels[i * 3 + 2] = fa_cv_to_u8(pixel.f1.f1);
    }
  }

  png_image png;
  memset(&png, 0, sizeof(png));
  png.version = PNG_IMAGE_VERSION;
  png.width = (png_uint_32)width;
  png.height = (png_uint_32)height;
  png.format = PNG_FORMAT_RGB;

  png_alloc_size_t encoded_len = 0;
  if (!png_image_write_to_memory(&png, NULL, &encoded_len, 0, pixels, 0, NULL)) {
    FaFaultable_Bytes out = FaFaultable_Bytes_fault(fa_fault_cstr("encode_png: failed to size output"));
    png_image_free(&png);
    free(pixels);
    return out;
  }
  char *encoded = (char *)malloc((size_t)encoded_len);
  if (!encoded) fa_die_alloc();
  if (!png_image_write_to_memory(&png, encoded, &encoded_len, 0, pixels, 0, NULL)) {
    char message[512];
    snprintf(message, sizeof(message), "encode_png: %s", png.message);
    png_image_free(&png);
    free(pixels);
    free(encoded);
    return FaFaultable_Bytes_fault(fa_fault_cstr(message));
  }
  png_image_free(&png);
  free(pixels);
  return FaFaultable_Bytes_ok(fa_bytes_owned(encoded, (size_t)encoded_len));
}

static uint16_t fa_cv_read_le16(const unsigned char *p) {
  return (uint16_t)p[0] | ((uint16_t)p[1] << 8);
}

static uint32_t fa_cv_read_le32(const unsigned char *p) {
  return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

static int32_t fa_cv_read_le_i32(const unsigned char *p) {
  return (int32_t)fa_cv_read_le32(p);
}

static void fa_cv_write_le16(unsigned char *p, uint16_t value) {
  p[0] = (unsigned char)(value & 0xff);
  p[1] = (unsigned char)((value >> 8) & 0xff);
}

static void fa_cv_write_le32(unsigned char *p, uint32_t value) {
  p[0] = (unsigned char)(value & 0xff);
  p[1] = (unsigned char)((value >> 8) & 0xff);
  p[2] = (unsigned char)((value >> 16) & 0xff);
  p[3] = (unsigned char)((value >> 24) & 0xff);
}

static FaCvImageResult fa_cv_decode_bmp(FaBytes bytes) {
  const unsigned char *data = (const unsigned char *)bytes.bytes;
  if (bytes.len < 54 || data[0] != 'B' || data[1] != 'M') {
    return fa_cv_image_fault_cstr("decode_bmp: invalid BMP header");
  }

  uint32_t pixel_offset = fa_cv_read_le32(data + 10);
  uint32_t dib_size = fa_cv_read_le32(data + 14);
  if (dib_size < 40 || bytes.len < 14 + (size_t)dib_size) {
    return fa_cv_image_fault_cstr("decode_bmp: unsupported DIB header");
  }
  int32_t width_raw = fa_cv_read_le_i32(data + 18);
  int32_t height_raw = fa_cv_read_le_i32(data + 22);
  uint16_t planes = fa_cv_read_le16(data + 26);
  uint16_t bpp = fa_cv_read_le16(data + 28);
  uint32_t compression = fa_cv_read_le32(data + 30);
  if (width_raw <= 0 || height_raw == 0 || height_raw == INT32_MIN) {
    return fa_cv_image_fault_cstr("decode_bmp: image dimensions must be positive");
  }
  if (planes != 1 || compression != 0 || (bpp != 24 && bpp != 32)) {
    return fa_cv_image_fault_cstr("decode_bmp: only uncompressed 24-bit and 32-bit BMP are supported");
  }

  size_t width = (size_t)width_raw;
  size_t height = height_raw < 0 ? (size_t)(-height_raw) : (size_t)height_raw;
  size_t count = 0;
  FaCvImageResult fault;
  if (!fa_cv_prepare_size("decode_bmp", width, height, &count, &fault)) return fault;
  size_t bytes_per_pixel = bpp / 8;
  size_t row_raw = 0;
  if (fa_cv_mul_overflows(width, bytes_per_pixel, &row_raw)) {
    return fa_cv_image_fault_cstr("decode_bmp: image is too large");
  }
  size_t row_stride = (row_raw + 3) & ~(size_t)3;
  size_t raster_len = 0;
  if (fa_cv_mul_overflows(row_stride, height, &raster_len)) {
    return fa_cv_image_fault_cstr("decode_bmp: image is too large");
  }
  size_t raster_end = 0;
  if (fa_cv_add_overflows((size_t)pixel_offset, raster_len, &raster_end) || raster_end > bytes.len) {
    return fa_cv_image_fault_cstr("decode_bmp: truncated pixel data");
  }

  bool top_down = height_raw < 0;
  FaCvImage image = fa_cv_image_new(width, height);
  for (size_t y = 0; y < height; y++) {
    size_t src_y = top_down ? y : height - 1 - y;
    const unsigned char *row = data + pixel_offset + src_y * row_stride;
    for (size_t x = 0; x < width; x++) {
      const unsigned char *px = row + x * bytes_per_pixel;
      FaCvPixel *pixel = fa_cv_pixel_at(&image, y, x);
      pixel->f0 = fa_cv_from_u8(px[2]);
      pixel->f1.f0 = fa_cv_from_u8(px[1]);
      pixel->f1.f1 = fa_cv_from_u8(px[0]);
    }
  }
  return fa_cv_image_ok(image);
}

static FaFaultable_Bytes fa_cv_encode_bmp(FaCvImage image) {
  size_t width = 0;
  size_t height = 0;
  size_t count = 0;
  FaFault fault;
  if (!fa_cv_prepare_image(image, "encode_bmp", &width, &height, &count, &fault)) {
    return FaFaultable_Bytes_fault(fault);
  }
  if (width > (size_t)INT32_MAX || height > (size_t)INT32_MAX) {
    return FaFaultable_Bytes_fault(fa_fault_cstr("encode_bmp: dimensions exceed BMP limits"));
  }
  size_t row_raw = 0;
  if (fa_cv_mul_overflows(width, 3, &row_raw)) {
    return FaFaultable_Bytes_fault(fa_fault_cstr("encode_bmp: image is too large"));
  }
  size_t row_stride = (row_raw + 3) & ~(size_t)3;
  size_t raster_len = 0;
  if (fa_cv_mul_overflows(row_stride, height, &raster_len)) {
    return FaFaultable_Bytes_fault(fa_fault_cstr("encode_bmp: image is too large"));
  }
  size_t total_len = 0;
  if (fa_cv_add_overflows(54, raster_len, &total_len) || total_len > UINT32_MAX) {
    return FaFaultable_Bytes_fault(fa_fault_cstr("encode_bmp: image is too large"));
  }

  unsigned char *out = (unsigned char *)calloc(total_len, 1);
  if (!out) fa_die_alloc();
  out[0] = 'B';
  out[1] = 'M';
  fa_cv_write_le32(out + 2, (uint32_t)total_len);
  fa_cv_write_le32(out + 10, 54);
  fa_cv_write_le32(out + 14, 40);
  fa_cv_write_le32(out + 18, (uint32_t)width);
  fa_cv_write_le32(out + 22, (uint32_t)height);
  fa_cv_write_le16(out + 26, 1);
  fa_cv_write_le16(out + 28, 24);
  fa_cv_write_le32(out + 34, (uint32_t)raster_len);

  for (size_t y = 0; y < height; y++) {
    size_t dst_y = height - 1 - y;
    unsigned char *row = out + 54 + dst_y * row_stride;
    for (size_t x = 0; x < width; x++) {
      FaCvPixel pixel = image.f1.items[y].items[x];
      row[x * 3] = fa_cv_to_u8(pixel.f1.f1);
      row[x * 3 + 1] = fa_cv_to_u8(pixel.f1.f0);
      row[x * 3 + 2] = fa_cv_to_u8(pixel.f0);
    }
  }
  (void)count;
  return FaFaultable_Bytes_ok(fa_bytes_owned((char *)out, total_len));
}

static bool fa_cv_is_space(unsigned char c) {
  return c == ' ' || c == '\n' || c == '\r' || c == '\t' || c == '\f' || c == '\v';
}

static void fa_cv_pnm_skip_ws_and_comments(const unsigned char *data, size_t len, size_t *pos) {
  while (*pos < len) {
    if (fa_cv_is_space(data[*pos])) {
      (*pos)++;
      continue;
    }
    if (data[*pos] == '#') {
      while (*pos < len && data[*pos] != '\n') (*pos)++;
      continue;
    }
    break;
  }
}

static bool fa_cv_pnm_uint(const unsigned char *data, size_t len, size_t *pos, uint32_t *value) {
  fa_cv_pnm_skip_ws_and_comments(data, len, pos);
  if (*pos >= len || data[*pos] < '0' || data[*pos] > '9') return false;
  uint32_t out = 0;
  while (*pos < len && data[*pos] >= '0' && data[*pos] <= '9') {
    uint32_t digit = (uint32_t)(data[*pos] - '0');
    if (out > (UINT32_MAX - digit) / 10) return false;
    out = out * 10 + digit;
    (*pos)++;
  }
  *value = out;
  return true;
}

static FaCvImageResult fa_cv_decode_pnm(FaBytes bytes) {
  const unsigned char *data = (const unsigned char *)bytes.bytes;
  if (bytes.len < 3 || data[0] != 'P' || (data[1] != '5' && data[1] != '6')) {
    return fa_cv_image_fault_cstr("decode_pnm: expected binary P5/P6 PNM");
  }
  bool gray = data[1] == '5';
  size_t pos = 2;
  uint32_t width_u = 0;
  uint32_t height_u = 0;
  uint32_t maxval = 0;
  if (!fa_cv_pnm_uint(data, bytes.len, &pos, &width_u)
      || !fa_cv_pnm_uint(data, bytes.len, &pos, &height_u)
      || !fa_cv_pnm_uint(data, bytes.len, &pos, &maxval)) {
    return fa_cv_image_fault_cstr("decode_pnm: invalid header");
  }
  if (maxval == 0 || maxval > 255) {
    return fa_cv_image_fault_cstr("decode_pnm: only maxval 1..255 is supported");
  }
  if (pos >= bytes.len || !fa_cv_is_space(data[pos])) {
    return fa_cv_image_fault_cstr("decode_pnm: missing raster separator");
  }
  pos++;

  size_t width = (size_t)width_u;
  size_t height = (size_t)height_u;
  size_t count = 0;
  FaCvImageResult fault;
  if (!fa_cv_prepare_size("decode_pnm", width, height, &count, &fault)) return fault;
  size_t channels = gray ? 1 : 3;
  size_t raster_len = 0;
  if (fa_cv_mul_overflows(count, channels, &raster_len)) {
    return fa_cv_image_fault_cstr("decode_pnm: image is too large");
  }
  size_t raster_end = 0;
  if (fa_cv_add_overflows(pos, raster_len, &raster_end) || raster_end > bytes.len) {
    return fa_cv_image_fault_cstr("decode_pnm: truncated pixel data");
  }

  FaCvImage image = fa_cv_image_new(width, height);
  for (size_t y = 0; y < height; y++) {
    for (size_t x = 0; x < width; x++) {
      size_t i = y * width + x;
      FaCvPixel *pixel = fa_cv_pixel_at(&image, y, x);
      if (gray) {
        double v = (double)data[pos + i] / (double)maxval;
        pixel->f0 = v;
        pixel->f1.f0 = v;
        pixel->f1.f1 = v;
      } else {
        pixel->f0 = (double)data[pos + i * 3] / (double)maxval;
        pixel->f1.f0 = (double)data[pos + i * 3 + 1] / (double)maxval;
        pixel->f1.f1 = (double)data[pos + i * 3 + 2] / (double)maxval;
      }
    }
  }
  return fa_cv_image_ok(image);
}

static FaFaultable_Bytes fa_cv_encode_ppm(FaCvImage image) {
  size_t width = 0;
  size_t height = 0;
  size_t count = 0;
  FaFault fault;
  if (!fa_cv_prepare_image(image, "encode_ppm", &width, &height, &count, &fault)) {
    return FaFaultable_Bytes_fault(fault);
  }
  char header[64];
  int header_len = snprintf(header, sizeof(header), "P6\n%zu %zu\n255\n", width, height);
  if (header_len < 0 || (size_t)header_len >= sizeof(header)) {
    return FaFaultable_Bytes_fault(fa_fault_cstr("encode_ppm: image dimensions are too large"));
  }
  size_t raster_len = 0;
  if (fa_cv_mul_overflows(count, 3, &raster_len)) {
    return FaFaultable_Bytes_fault(fa_fault_cstr("encode_ppm: image is too large"));
  }
  size_t total_len = 0;
  if (fa_cv_add_overflows((size_t)header_len, raster_len, &total_len)) {
    return FaFaultable_Bytes_fault(fa_fault_cstr("encode_ppm: image is too large"));
  }
  char *out = (char *)malloc(total_len + 1);
  if (!out) fa_die_alloc();
  memcpy(out, header, (size_t)header_len);
  unsigned char *pixels = (unsigned char *)out + header_len;
  for (size_t y = 0; y < height; y++) {
    for (size_t x = 0; x < width; x++) {
      size_t i = y * width + x;
      FaCvPixel pixel = image.f1.items[y].items[x];
      pixels[i * 3] = fa_cv_to_u8(pixel.f0);
      pixels[i * 3 + 1] = fa_cv_to_u8(pixel.f1.f0);
      pixels[i * 3 + 2] = fa_cv_to_u8(pixel.f1.f1);
    }
  }
  out[total_len] = '\0';
  return FaFaultable_Bytes_ok(fa_bytes_owned(out, total_len));
}

static FaFaultable_Bytes fa_cv_encode_pgm(FaCvImage image) {
  size_t width = 0;
  size_t height = 0;
  size_t count = 0;
  FaFault fault;
  if (!fa_cv_prepare_image(image, "encode_pgm", &width, &height, &count, &fault)) {
    return FaFaultable_Bytes_fault(fault);
  }
  char header[64];
  int header_len = snprintf(header, sizeof(header), "P5\n%zu %zu\n255\n", width, height);
  if (header_len < 0 || (size_t)header_len >= sizeof(header)) {
    return FaFaultable_Bytes_fault(fa_fault_cstr("encode_pgm: image dimensions are too large"));
  }
  size_t total_len = 0;
  if (fa_cv_add_overflows((size_t)header_len, count, &total_len)) {
    return FaFaultable_Bytes_fault(fa_fault_cstr("encode_pgm: image is too large"));
  }
  char *out = (char *)malloc(total_len + 1);
  if (!out) fa_die_alloc();
  memcpy(out, header, (size_t)header_len);
  unsigned char *pixels = (unsigned char *)out + header_len;
  for (size_t y = 0; y < height; y++) {
    for (size_t x = 0; x < width; x++) {
      size_t i = y * width + x;
      pixels[i] = fa_cv_luma(image.f1.items[y].items[x]);
    }
  }
  out[total_len] = '\0';
  return FaFaultable_Bytes_ok(fa_bytes_owned(out, total_len));
}

static FaCvImageResult fa_cv_decode(FaBytes bytes) {
  const unsigned char *data = (const unsigned char *)bytes.bytes;
  if (bytes.len >= 3 && data[0] == 0xff && data[1] == 0xd8 && data[2] == 0xff) {
    return fa_cv_decode_jpeg(bytes);
  }
  if (bytes.len >= 8
      && data[0] == 137
      && data[1] == 'P'
      && data[2] == 'N'
      && data[3] == 'G'
      && data[4] == 13
      && data[5] == 10
      && data[6] == 26
      && data[7] == 10) {
    return fa_cv_decode_png(bytes);
  }
  if (bytes.len >= 2 && data[0] == 'B' && data[1] == 'M') {
    return fa_cv_decode_bmp(bytes);
  }
  if (bytes.len >= 2 && data[0] == 'P' && (data[1] == '5' || data[1] == '6')) {
    return fa_cv_decode_pnm(bytes);
  }
  return fa_cv_image_fault_cstr("decode: unsupported image format; expected JPEG, PNG, BMP, PGM, or PPM");
}
