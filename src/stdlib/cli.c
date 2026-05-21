#include "runtime.h"

static FaSeq_Bytes fa_argv(FaArgs args) {
  int64_t count = args.argc > 1 ? args.argc - 1 : 0;
  FaSeq_Bytes out = FaSeq_Bytes_new((size_t)count);
  for (int64_t i = 0; i < count; i++) out.items[i] = fa_bytes_literal(args.argv[i + 1], strlen(args.argv[i + 1]));
  return out;
}

static bool fa_arg_equals(char *arg, FaBytes expected) {
  return strlen(arg) == expected.len && memcmp(arg, expected.bytes, expected.len) == 0;
}

static bool fa_arg_has_equals_value(char *arg, FaBytes flag) {
  size_t len = strlen(arg);
  return len > flag.len && arg[flag.len] == '=' && memcmp(arg, flag.bytes, flag.len) == 0;
}

static bool fa_flag_present(FaArgs args, FaBytes flag) {
  for (int i = 1; i < args.argc; i++) {
    if (fa_arg_equals(args.argv[i], flag) || fa_arg_has_equals_value(args.argv[i], flag)) return true;
  }
  return false;
}

static FaFaultable_Bytes fa_flag_value(FaArgs args, FaBytes flag) {
  for (int i = 1; i < args.argc; i++) {
    if (fa_arg_has_equals_value(args.argv[i], flag)) {
      char *value = args.argv[i] + flag.len + 1;
      return FaFaultable_Bytes_ok(fa_bytes_literal(value, strlen(value)));
    }
    if (fa_arg_equals(args.argv[i], flag)) {
      if (i + 1 >= args.argc) return FaFaultable_Bytes_fault(fa_fault_cstr("flag_value: flag has no value"));
      return FaFaultable_Bytes_ok(fa_bytes_literal(args.argv[i + 1], strlen(args.argv[i + 1])));
    }
  }
  return FaFaultable_Bytes_fault(fa_fault_cstr("flag_value: flag is not present"));
}
