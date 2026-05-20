static FaValue *fa_select(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "select");
    if (seq->count != 3 || !seq->items[0] || seq->items[0]->kind != FA_BOOL) {
        fputs("flowarrow runtime: select expected (Bool, T, T)\n", stderr);
        exit(65);
    }
    return seq->items[0]->b ? seq->items[1] : seq->items[2];
}

static FaValue *fa_range_step(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "range_step");
    if (seq->count != 3) {
        fputs("flowarrow runtime: range_step expected three Int inputs\n", stderr);
        exit(65);
    }
    int64_t start = fa_expect_int(seq->items[0], "range_step");
    int64_t stop = fa_expect_int(seq->items[1], "range_step");
    int64_t step = fa_expect_int(seq->items[2], "range_step");
    if (step == 0) {
        fputs("flowarrow runtime: range_step step cannot be zero\n", stderr);
        exit(65);
    }
    int64_t count = 0;
    for (int64_t i = start; step > 0 ? i < stop : i > stop; i += step) {
        count++;
    }
    FaValue *out = fa_seq_new(count);
    int64_t index = 0;
    for (int64_t i = start; step > 0 ? i < stop : i > stop; i += step) {
        fa_seq_set(out, index++, fa_int(i));
    }
    return out;
}
