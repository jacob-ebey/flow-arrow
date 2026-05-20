static bool fa_try_parse_real(FaValue *input, double *out) {
    FaValue *bytes = fa_expect_bytes(input, "parse_real");
    char *copy = fa_copy_bytes(bytes->bytes, bytes->len);
    char *start = copy;
    while (isspace((unsigned char)*start)) {
        start++;
    }
    errno = 0;
    char *end = NULL;
    double value = strtod(start, &end);
    while (end && isspace((unsigned char)*end)) {
        end++;
    }
    bool ok = !(start == end || errno == ERANGE || !end || *end != '\0');
    free(copy);
    if (ok) {
        *out = value;
    }
    return ok;
}

FaValue *fa_parse_real(FaValue *input) {
    FaValue *bytes = fa_expect_bytes(input, "parse_real");
    double value = 0.0;
    if (!fa_try_parse_real(input, &value)) {
        fprintf(stderr, "flowarrow runtime: invalid Real input `%.*s`\n", (int)bytes->len, bytes->bytes);
        exit(65);
    }
    return fa_real(value);
}

FaValue *fa_format_real(FaValue *input) {
    char buf[64];
    snprintf(buf, sizeof(buf), "%.15g", fa_expect_real(input, "format_real"));
    return fa_cstr(buf);
}
