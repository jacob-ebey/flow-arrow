#include "runtime.h"

static FaSeq_Int fa_range_step(int64_t start, int64_t stop, int64_t step) {
  if (step == 0) fa_die_usage("range_step: step cannot be zero");
  __int128 distance;
  __int128 stride;
  if (step > 0) {
    if (start >= stop) return FaSeq_Int_new(0);
    distance = (__int128)stop - (__int128)start;
    stride = (__int128)step;
  } else {
    if (start <= stop) return FaSeq_Int_new(0);
    distance = (__int128)start - (__int128)stop;
    stride = -(__int128)step;
  }
  __int128 wide_count = (distance + stride - 1) / stride;
  if (wide_count > (__int128)SIZE_MAX) fa_die_usage("range_step: range is too large");
  size_t count = (size_t)wide_count;
  FaSeq_Int out = FaSeq_Int_new(count);
  __int128 value = (__int128)start;
  for (size_t index = 0; index < count; index++) {
    if (value < (__int128)INT64_MIN || value > (__int128)INT64_MAX) {
      fa_die_usage("range_step: integer overflow");
    }
    out.items[index] = (int64_t)value;
    value += (__int128)step;
  }
  return out;
}
