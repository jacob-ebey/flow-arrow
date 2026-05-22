#ifndef FLOWARROW_C_INTEROP_NATIVE_MATH_H
#define FLOWARROW_C_INTEROP_NATIVE_MATH_H

#include <stdint.h>
#include <stddef.h>

typedef struct {
  char *bytes;
  size_t len;
} FaBytes;

int64_t fa_native_score(int64_t value);
FaBytes fa_native_label(int64_t score);

#endif
