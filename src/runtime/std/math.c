static int64_t fa_checked_integer_add(int64_t left, int64_t right, const char *op) {
    int64_t result = 0;
    if (__builtin_add_overflow(left, right, &result)) {
        fprintf(stderr, "flowarrow runtime: %s overflow\n", op);
        exit(65);
    }
    return result;
}

static int64_t fa_checked_integer_sub(int64_t left, int64_t right, const char *op) {
    int64_t result = 0;
    if (__builtin_sub_overflow(left, right, &result)) {
        fprintf(stderr, "flowarrow runtime: %s overflow\n", op);
        exit(65);
    }
    return result;
}

static FaValue *fa_add(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "add");
    if (seq->count != 2) {
        fputs("flowarrow runtime: add expected two inputs\n", stderr);
        exit(65);
    }
    if (seq->items[0] && seq->items[0]->kind == FA_INT
        && seq->items[1] && seq->items[1]->kind == FA_INT) {
        int64_t left = fa_expect_int(seq->items[0], "add");
        int64_t right = fa_expect_int(seq->items[1], "add");
        return fa_int(fa_checked_integer_add(left, right, "add"));
    }
    return fa_real(
        fa_expect_number(seq->items[0], "add") + fa_expect_number(seq->items[1], "add"));
}

static FaValue *fa_sub(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "sub");
    if (seq->count != 2) {
        fputs("flowarrow runtime: sub expected two inputs\n", stderr);
        exit(65);
    }
    if (seq->items[0] && seq->items[0]->kind == FA_INT
        && seq->items[1] && seq->items[1]->kind == FA_INT) {
        int64_t left = fa_expect_int(seq->items[0], "sub");
        int64_t right = fa_expect_int(seq->items[1], "sub");
        return fa_int(fa_checked_integer_sub(left, right, "sub"));
    }
    return fa_real(
        fa_expect_number(seq->items[0], "sub") - fa_expect_number(seq->items[1], "sub"));
}

static FaValue *fa_eq(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "eq");
    if (seq->count != 2) {
        fputs("flowarrow runtime: eq expected two inputs\n", stderr);
        exit(65);
    }
    if (seq->items[0] && seq->items[0]->kind == FA_INT
        && seq->items[1] && seq->items[1]->kind == FA_INT) {
        return fa_bool(fa_expect_int(seq->items[0], "eq") == fa_expect_int(seq->items[1], "eq"));
    }
    return fa_bool(
        fa_expect_number(seq->items[0], "eq") == fa_expect_number(seq->items[1], "eq"));
}

static FaValue *fa_max(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "max");
    if (seq->count != 2) {
        fputs("flowarrow runtime: max expected two inputs\n", stderr);
        exit(65);
    }
    if (seq->items[0] && seq->items[0]->kind == FA_INT
        && seq->items[1] && seq->items[1]->kind == FA_INT) {
        int64_t left = fa_expect_int(seq->items[0], "max");
        int64_t right = fa_expect_int(seq->items[1], "max");
        return fa_int(left > right ? left : right);
    }
    double left = fa_expect_number(seq->items[0], "max");
    double right = fa_expect_number(seq->items[1], "max");
    return fa_real(left > right ? left : right);
}

static FaValue *fa_mul(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "mul");
    if (seq->count != 2) {
        fputs("flowarrow runtime: mul expected two inputs\n", stderr);
        exit(65);
    }
    if (seq->items[0] && seq->items[0]->kind == FA_INT
        && seq->items[1] && seq->items[1]->kind == FA_INT) {
        int64_t left = fa_expect_int(seq->items[0], "mul");
        int64_t right = fa_expect_int(seq->items[1], "mul");
        int64_t result = 0;
        if (__builtin_mul_overflow(left, right, &result)) {
            fprintf(stderr, "flowarrow runtime: mul overflow\n");
            exit(65);
        }
        return fa_int(result);
    }
    return fa_real(
        fa_expect_number(seq->items[0], "mul") * fa_expect_number(seq->items[1], "mul"));
}

static FaValue *fa_div(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "div");
    if (seq->count != 2) {
        fputs("flowarrow runtime: div expected two inputs\n", stderr);
        exit(65);
    }
    if (seq->items[0] && seq->items[0]->kind == FA_INT
        && seq->items[1] && seq->items[1]->kind == FA_INT) {
        int64_t left = fa_expect_int(seq->items[0], "div");
        int64_t right = fa_expect_int(seq->items[1], "div");
        if (right == 0) {
            fputs("flowarrow runtime: div by zero\n", stderr);
            exit(65);
        }
        return fa_int(left / right);
    }
    double right = fa_expect_number(seq->items[1], "div");
    if (right == 0.0) {
        fputs("flowarrow runtime: div by zero\n", stderr);
        exit(65);
    }
    return fa_real(fa_expect_number(seq->items[0], "div") / right);
}

static FaValue *fa_rem(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "rem");
    if (seq->count != 2) {
        fputs("flowarrow runtime: rem expected two inputs\n", stderr);
        exit(65);
    }
    if (seq->items[0] && seq->items[0]->kind == FA_INT
        && seq->items[1] && seq->items[1]->kind == FA_INT) {
        int64_t left = fa_expect_int(seq->items[0], "rem");
        int64_t right = fa_expect_int(seq->items[1], "rem");
        if (right == 0) {
            fputs("flowarrow runtime: rem by zero\n", stderr);
            exit(65);
        }
        return fa_int(left % right);
    }
    double right = fa_expect_number(seq->items[1], "rem");
    if (right == 0.0) {
        fputs("flowarrow runtime: rem by zero\n", stderr);
        exit(65);
    }
    return fa_real(fmod(fa_expect_number(seq->items[0], "rem"), right));
}

static FaValue *fa_lt(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "lt");
    if (seq->count != 2) {
        fputs("flowarrow runtime: lt expected two inputs\n", stderr);
        exit(65);
    }
    if (seq->items[0] && seq->items[0]->kind == FA_INT
        && seq->items[1] && seq->items[1]->kind == FA_INT) {
        return fa_bool(fa_expect_int(seq->items[0], "lt") < fa_expect_int(seq->items[1], "lt"));
    }
    return fa_bool(fa_expect_number(seq->items[0], "lt") < fa_expect_number(seq->items[1], "lt"));
}

static FaValue *fa_gt(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "gt");
    if (seq->count != 2) {
        fputs("flowarrow runtime: gt expected two inputs\n", stderr);
        exit(65);
    }
    if (seq->items[0] && seq->items[0]->kind == FA_INT
        && seq->items[1] && seq->items[1]->kind == FA_INT) {
        return fa_bool(fa_expect_int(seq->items[0], "gt") > fa_expect_int(seq->items[1], "gt"));
    }
    return fa_bool(fa_expect_number(seq->items[0], "gt") > fa_expect_number(seq->items[1], "gt"));
}

static FaValue *fa_le(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "le");
    if (seq->count != 2) {
        fputs("flowarrow runtime: le expected two inputs\n", stderr);
        exit(65);
    }
    if (seq->items[0] && seq->items[0]->kind == FA_INT
        && seq->items[1] && seq->items[1]->kind == FA_INT) {
        return fa_bool(fa_expect_int(seq->items[0], "le") <= fa_expect_int(seq->items[1], "le"));
    }
    return fa_bool(fa_expect_number(seq->items[0], "le") <= fa_expect_number(seq->items[1], "le"));
}

static FaValue *fa_ge(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "ge");
    if (seq->count != 2) {
        fputs("flowarrow runtime: ge expected two inputs\n", stderr);
        exit(65);
    }
    if (seq->items[0] && seq->items[0]->kind == FA_INT
        && seq->items[1] && seq->items[1]->kind == FA_INT) {
        return fa_bool(fa_expect_int(seq->items[0], "ge") >= fa_expect_int(seq->items[1], "ge"));
    }
    return fa_bool(fa_expect_number(seq->items[0], "ge") >= fa_expect_number(seq->items[1], "ge"));
}
