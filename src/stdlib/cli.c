#include "runtime.h"

static FaSeq_Bytes fa_argv(FaArgs args) {
  int64_t count = args.argc > 1 ? args.argc - 1 : 0;
  FaSeq_Bytes out = FaSeq_Bytes_new((size_t)count);
  for (int64_t i = 0; i < count; i++) out.items[i] = fa_bytes_literal(args.argv[i + 1], strlen(args.argv[i + 1]));
  return out;
}
