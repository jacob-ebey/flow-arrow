FaValue *fa_format_int(FaValue *input) {
    char buf[64];
    snprintf(buf, sizeof(buf), "%lld", (long long)fa_expect_int(input, "format_int"));
    return fa_cstr(buf);
}

FaValue *fa_parse_int(FaValue *input) {
    FaValue *bytes = fa_expect_bytes(input, "parse_int");
    char *copy = fa_copy_bytes(bytes->bytes, bytes->len);
    char *start = copy;
    while (isspace((unsigned char)*start)) {
        start++;
    }
    errno = 0;
    char *end = NULL;
    long long value = strtoll(start, &end, 10);
    while (end && isspace((unsigned char)*end)) {
        end++;
    }
    if (start == end || errno == ERANGE || !end || *end != '\0') {
        fprintf(stderr, "flowarrow runtime: invalid Int input `%.*s`\n", (int)bytes->len, bytes->bytes);
        exit(65);
    }
    free(copy);
    return fa_int((int64_t)value);
}
