#include <jpeglib.h>
#include <setjmp.h>

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

static FaFaultable_Tuple_Tuple_Int_Int_Seq_Tuple_Int_Tuple_Int_Int fa_cv_image_fault_cstr(const char *message) {
  FaFaultable_Tuple_Tuple_Int_Int_Seq_Tuple_Int_Tuple_Int_Int out;
  out.is_fault = true;
  out.fault = fa_fault_cstr(message);
  return out;
}

static FaFaultable_Tuple_Tuple_Int_Int_Seq_Tuple_Int_Tuple_Int_Int fa_cv_decode_jpeg(FaBytes bytes) {
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

  size_t width = (size_t)cinfo.output_width;
  size_t height = (size_t)cinfo.output_height;
  if (height != 0 && width > SIZE_MAX / height) {
    jpeg_destroy_decompress(&cinfo);
    return fa_cv_image_fault_cstr("decode_jpeg: image is too large");
  }
  size_t count = width * height;
  if (count != 0 && count > SIZE_MAX / sizeof(FaTuple_Int_Tuple_Int_Int)) {
    jpeg_destroy_decompress(&cinfo);
    return fa_cv_image_fault_cstr("decode_jpeg: image is too large");
  }

  FaTuple_Tuple_Int_Int_Seq_Tuple_Int_Tuple_Int_Int image;
  image.f0.f0 = (int64_t)width;
  image.f0.f1 = (int64_t)height;
  image.f1 = FaSeq_Tuple_Int_Tuple_Int_Int_new(count);

  size_t row_stride = width * (size_t)cinfo.output_components;
  JSAMPARRAY row = (*cinfo.mem->alloc_sarray)((j_common_ptr)&cinfo, JPOOL_IMAGE, row_stride, 1);
  while (cinfo.output_scanline < cinfo.output_height) {
    JDIMENSION y = cinfo.output_scanline;
    jpeg_read_scanlines(&cinfo, row, 1);
    for (size_t x = 0; x < width; x++) {
      size_t index = (size_t)y * width + x;
      image.f1.items[index].f0 = row[0][x * 3];
      image.f1.items[index].f1.f0 = row[0][x * 3 + 1];
      image.f1.items[index].f1.f1 = row[0][x * 3 + 2];
    }
  }
  jpeg_finish_decompress(&cinfo);
  jpeg_destroy_decompress(&cinfo);

  FaFaultable_Tuple_Tuple_Int_Int_Seq_Tuple_Int_Tuple_Int_Int out;
  out.is_fault = false;
  out.value = image;
  return out;
}

static bool fa_cv_channel_ok(int64_t value) {
  return value >= 0 && value <= 255;
}

static FaFaultable_Bytes fa_cv_encode_jpeg(FaTuple_Tuple_Int_Int_Seq_Tuple_Int_Tuple_Int_Int image) {
  if (image.f0.f0 <= 0 || image.f0.f1 <= 0) {
    return FaFaultable_Bytes_fault(fa_fault_cstr("encode_jpeg: image dimensions must be positive"));
  }
  size_t width = (size_t)image.f0.f0;
  size_t height = (size_t)image.f0.f1;
  if (height != 0 && width > SIZE_MAX / height) {
    return FaFaultable_Bytes_fault(fa_fault_cstr("encode_jpeg: image is too large"));
  }
  size_t expected = width * height;
  if (image.f1.count != expected) {
    return FaFaultable_Bytes_fault(fa_fault_cstr("encode_jpeg: pixel count does not match dimensions"));
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
      size_t index = y * width + x;
      int64_t r = image.f1.items[index].f0;
      int64_t g = image.f1.items[index].f1.f0;
      int64_t b = image.f1.items[index].f1.f1;
      if (!fa_cv_channel_ok(r) || !fa_cv_channel_ok(g) || !fa_cv_channel_ok(b)) {
        jpeg_destroy_compress(&cinfo);
        free(row);
        free(encoded);
        return FaFaultable_Bytes_fault(fa_fault_cstr("encode_jpeg: pixel channel outside 0..255"));
      }
      row[x * 3] = (unsigned char)r;
      row[x * 3 + 1] = (unsigned char)g;
      row[x * 3 + 2] = (unsigned char)b;
    }
    JSAMPROW row_pointer[1] = { row };
    jpeg_write_scanlines(&cinfo, row_pointer, 1);
  }
  jpeg_finish_compress(&cinfo);
  jpeg_destroy_compress(&cinfo);
  free(row);

  return FaFaultable_Bytes_ok(fa_bytes_owned((char *)encoded, (size_t)encoded_len));
}
