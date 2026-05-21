#include "runtime.h"

static FaSeq_Int fa_range_step(int64_t start, int64_t stop, int64_t step) {
  if (step == 0) fa_die_usage("range_step: step cannot be zero");
  size_t count = 0;
  for (int64_t i = start; step > 0 ? i < stop : i > stop; i += step) count++;
  FaSeq_Int out = FaSeq_Int_new(count);
  size_t index = 0;
  for (int64_t i = start; step > 0 ? i < stop : i > stop; i += step) out.items[index++] = i;
  return out;
}
