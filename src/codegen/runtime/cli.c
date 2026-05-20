static FaValue fa_args(int argc, char **argv) {
  int64_t count = argc > 1 ? (int64_t)argc - 1 : 0;
  FaValue args = fa_seq_new((size_t)count);
  for (int64_t i = 0; i < count; i++) {
    fa_seq_set(&args, (size_t)i, fa_bytes_from_slice(argv[i + 1], strlen(argv[i + 1])));
  }
  return args;
}

static FaValue fa_builtin_argv(FaValue input) {
  return fa_expect_seq(input, "argv");
}

