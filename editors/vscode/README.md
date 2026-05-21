# FlowArrow VS Code extension

This extension adds syntax highlighting and editor language configuration for
FlowArrow source files.

## Features

- Registers `.flow` and `.flowarrow` files as FlowArrow.
- Highlights FlowArrow declarations, type aliases, imports, combinators,
  comments, strings, literals, types, punctuation, and the `->` flow arrow.
- Gives `$`-prefixed variables a dedicated scope so they can be colored
  differently from node references and declaration names.
- Gives built-in combinators such as `repeat`, `map`, `select`, and
  `range_step` a dedicated builtin scope so they can be colored separately
  from variables and ordinary node references.
- Marks forbidden keywords and assignment-like operators from `docs/syntax.md`
  as illegal tokens.
- Configures comments, bracket matching, auto-closing pairs, surrounding pairs,
  and region folding.

## Run locally

From this directory:

```sh
code --extensionDevelopmentPath="$(pwd)"
```

## Package locally

If you have the VS Code extension packaging tool available:

```sh
npx @vscode/vsce package
code --install-extension flowarrow-vscode-0.1.0.vsix
```
