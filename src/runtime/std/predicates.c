FaValue *fa_not_empty(FaValue *input) {
    FaValue *bytes = fa_expect_bytes(input, "not_empty");
    return fa_bool(bytes->len > 0);
}

static FaValue *fa_is_empty(FaValue *input) {
    FaValue *bytes = fa_expect_bytes(input, "is_empty");
    return fa_bool(bytes->len == 0);
}

static FaValue *fa_and(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "and");
    if (seq->count != 2 || !seq->items[0] || seq->items[0]->kind != FA_BOOL
        || !seq->items[1] || seq->items[1]->kind != FA_BOOL) {
        fputs("flowarrow runtime: and expected (Bool, Bool)\n", stderr);
        exit(65);
    }
    return fa_bool(seq->items[0]->b && seq->items[1]->b);
}

static FaValue *fa_or(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "or");
    if (seq->count != 2 || !seq->items[0] || seq->items[0]->kind != FA_BOOL
        || !seq->items[1] || seq->items[1]->kind != FA_BOOL) {
        fputs("flowarrow runtime: or expected (Bool, Bool)\n", stderr);
        exit(65);
    }
    return fa_bool(seq->items[0]->b || seq->items[1]->b);
}

static FaValue *fa_xor(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "xor");
    if (seq->count != 2 || !seq->items[0] || seq->items[0]->kind != FA_BOOL
        || !seq->items[1] || seq->items[1]->kind != FA_BOOL) {
        fputs("flowarrow runtime: xor expected (Bool, Bool)\n", stderr);
        exit(65);
    }
    return fa_bool(seq->items[0]->b != seq->items[1]->b);
}

FaValue *fa_not(FaValue *input) {
    if (!input || input->kind != FA_BOOL) {
        fputs("flowarrow runtime: not expected Bool\n", stderr);
        exit(65);
    }
    return fa_bool(!input->b);
}

static FaValue *fa_all(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "all");
    for (size_t i = 0; i < seq->count; i++) {
        if (!seq->items[i] || seq->items[i]->kind != FA_BOOL) {
            fputs("flowarrow runtime: all expected Seq[Bool]\n", stderr);
            exit(65);
        }
        if (!seq->items[i]->b) {
            return fa_bool(false);
        }
    }
    return fa_bool(true);
}

static FaValue *fa_any(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "any");
    for (size_t i = 0; i < seq->count; i++) {
        if (!seq->items[i] || seq->items[i]->kind != FA_BOOL) {
            fputs("flowarrow runtime: any expected Seq[Bool]\n", stderr);
            exit(65);
        }
        if (seq->items[i]->b) {
            return fa_bool(true);
        }
    }
    return fa_bool(false);
}
