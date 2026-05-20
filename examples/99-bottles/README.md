# 99-bottles

The canonical "99 bottles of beer on the wall" — written without
loops, mutation, or sequencing.

```text
$ flowarrow run main.flow
99 bottles of beer on the wall, 99 bottles of beer.
Take one down and pass it around, 98 bottles of beer on the wall.

98 bottles of beer on the wall, 98 bottles of beer.
Take one down and pass it around, 97 bottles of beer on the wall.

...

1 bottle of beer on the wall, 1 bottle of beer.
Take one down and pass it around, 0 bottles of beer on the wall.

No more bottles of beer on the wall, no more bottles of beer.
Go to the store and buy some more, 99 bottles of beer on the wall.
```

## Why this example matters

A classic imperative benchmark — usually a `for i in 99..0` loop —
expressed as a pure dataflow graph:

1. **No loop.** The verse indices are produced by `range_step(99, 0, -1)`.
   The compiler sees a sequence of length 99 whose elements are
   independent.

2. **Per-element work in parallel.** `map verse_for` evaluates all 99
   verses concurrently. Each verse depends only on its own index.
   On unlimited processors, the critical path is one `verse_for`
   call plus a `O(log 99)` concat tree.

3. **Order preserved via associative reduce.** Byte concatenation is
   associative (but not commutative). `reduce concat_bytes` is
   compiled as a balanced tree; the result is identical to a
   left-fold, but the work is parallel.

4. **No conditional branch for the final verse.** The "no more
   bottles" stanza is a separate pure node (`final_verse_node`)
   joined to the body with `concat2`. There is no `if i == 0` —
   that path would hide control flow inside the graph.

5. **Pluralisation via `select`, not branching.** "1 bottle" vs
   "N bottles" is chosen with the pure `select` combinator:
   both candidate strings are ordinary graph inputs, and the
   predicate `eq(n, 1)` picks one. The scheduler still sees
   the full dependency graph.

## What it does *not* require

- No loop construct.
- No mutation or counter variable.
- No `if` / `else`.
- No recursion.
- No statement ordering: rearranging the chains in `main` produces
  an identical program.
