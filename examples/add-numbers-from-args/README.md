# add-numbers-from-args

```text
$ flowarrow build main.flow
$ build/<target>/main 1.5 2.5 3
7
```

This example mirrors `add-numbers-from-stdin`, but uses `std.cli.argv`
to read command-line arguments instead of `stdin`. The executable name is
not included in `argv`, so `program 1.5 2.5 3` parses exactly three
numbers.
