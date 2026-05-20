FaValue *fa_args(int argc, char **argv) {
    int64_t count = argc > 1 ? (int64_t)argc - 1 : 0;
    FaValue *args = fa_seq_new(count);
    for (int64_t i = 0; i < count; i++) {
        fa_seq_set(args, i, fa_cstr(argv[i + 1]));
    }
    return args;
}

static FaValue *fa_argv(FaValue *input) {
    return fa_expect_seq(input, "argv");
}
