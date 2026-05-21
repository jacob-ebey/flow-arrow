# grep

A small grep-like CLI. It reads a search byte string and one or more file,
directory, or glob targets from positional command-line arguments, walks each
target, reads regular files, and writes each matching line as
`filepath:line-number:line`. The output ends with totals for files walked after
glob expansion and files scanned after reading.

```text
$ flowarrow run main.flow needle ./src "*.flow"
```

## Why this example matters

It shows the basic file-search pipeline without hiding boundary effects inside
higher-order functions:

1. `argv` provides the search needle and target list.
2. The first target determines a search root. If `root/.gitignore` exists, the
   example reads it, turns active lines into ignore filters, and passes those
   filters into the per-file grep node.
3. `map walk_files` expands each target, including glob patterns, into sorted
   regular files. The path list is filtered by the loaded ignore values before
   scanning.
4. `open_file -> stream.to_seq -> concat_bytes` streams each file's bytes at the
   boundary before materializing the current file for line matching.
5. Pure graph stages split file contents into lines, pair each line with a
   1-based line number, filter with `std.bytes.contains`, and format matching
   lines for stdout.
6. Walked and scanned file counts are appended as summary lines. The walked
   count comes from the raw flattened `walk_files` output; the scanned count is
   the number of paths that remain after applying ignore filters.

The matching is byte-oriented and literal: there are no regular expressions,
case folding, or context lines.
