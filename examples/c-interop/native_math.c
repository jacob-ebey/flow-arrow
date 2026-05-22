#include "native_math.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int64_t fa_native_score(int64_t value) {
  return value * 7 + 3;
}

FaBytes fa_native_label(int64_t score) {
  char buffer[64];
  int len = snprintf(buffer, sizeof(buffer), "native-score:%lld", (long long)score);
  if (len < 0) {
    return (FaBytes){ .bytes = NULL, .len = 0 };
  }

  char *bytes = malloc((size_t)len + 1);
  if (!bytes) {
    return (FaBytes){ .bytes = NULL, .len = 0 };
  }
  memcpy(bytes, buffer, (size_t)len);
  bytes[len] = '\0';
  return (FaBytes){ .bytes = bytes, .len = (size_t)len };
}
