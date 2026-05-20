# FlowArrow VS Code extension

This extension adds syntax highlighting and editor language configuration for
FlowArrow source files.

## Features

- Registers `.flow` and `.flowarrow` files as FlowArrow.
- Highlights FlowArrow declarations, imports, combinators, comments, strings,
  literals, types, punctuation, and the `->` flow arrow.
- Marks syntax listed as forbidden in `docs/syntax.md` as illegal tokens.
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
