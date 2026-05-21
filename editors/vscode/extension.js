const cp = require("child_process");
const fs = require("fs");
const path = require("path");
const vscode = require("vscode");

let client;
let diagnostics;

async function activate(context) {
  diagnostics = vscode.languages.createDiagnosticCollection("flowarrow");
  context.subscriptions.push(diagnostics);

  const output = vscode.window.createOutputChannel("FlowArrow");
  context.subscriptions.push(output);

  client = new LspClient(context, output, diagnostics);
  context.subscriptions.push(client);
  client.start();

  context.subscriptions.push(
    vscode.workspace.onDidOpenTextDocument((document) => client.didOpen(document)),
    vscode.workspace.onDidChangeTextDocument((event) => client.didChange(event.document)),
    vscode.workspace.onDidCloseTextDocument((document) => client.didClose(document)),
    vscode.languages.registerCompletionItemProvider(
      "flowarrow",
      new CompletionProvider(client),
      ".",
      "$",
    ),
    vscode.languages.registerDefinitionProvider("flowarrow", new DefinitionProvider(client)),
    vscode.languages.registerHoverProvider("flowarrow", new HoverProvider(client)),
    vscode.languages.registerDocumentSymbolProvider(
      "flowarrow",
      new DocumentSymbolProvider(client),
    ),
    vscode.languages.registerDocumentFormattingEditProvider(
      "flowarrow",
      new FormattingProvider(context, output),
    ),
  );

  for (const document of vscode.workspace.textDocuments) {
    client.didOpen(document);
  }
}

function deactivate() {
  if (client) {
    return client.stop();
  }
  return undefined;
}

class LspClient {
  constructor(context, output, diagnosticCollection) {
    this.context = context;
    this.output = output;
    this.diagnostics = diagnosticCollection;
    this.nextId = 1;
    this.pending = new Map();
    this.buffer = Buffer.alloc(0);
    this.ready = false;
    this.stopped = false;
  }

  start() {
    const command = resolveServerPath(this.context);
    this.output.appendLine(`Starting ${command} lsp`);
    this.process = cp.spawn(command, ["lsp"], {
      cwd: workspaceRoot() || this.context.extensionPath,
      stdio: ["pipe", "pipe", "pipe"],
    });

    this.process.stdout.on("data", (chunk) => this.handleData(chunk));
    this.process.stderr.on("data", (chunk) => this.output.append(chunk.toString()));
    this.process.on("error", (error) => {
      this.output.appendLine(`Failed to start FlowArrow language server: ${error.message}`);
    });
    this.process.on("exit", (code, signal) => {
      this.ready = false;
      if (!this.stopped) {
        this.output.appendLine(`FlowArrow language server exited: code=${code} signal=${signal}`);
      }
    });

    this.request("initialize", {
      processId: process.pid,
      rootUri: workspaceRootUri(),
      capabilities: {},
      workspaceFolders: vscode.workspace.workspaceFolders?.map((folder) => ({
        uri: folder.uri.toString(),
        name: folder.name,
      })),
    })
      .then(() => {
        this.ready = true;
        this.notify("initialized", {});
        for (const document of vscode.workspace.textDocuments) {
          this.didOpen(document);
        }
      })
      .catch((error) => this.output.appendLine(error.message));
  }

  async stop() {
    this.stopped = true;
    if (!this.process || this.process.killed) {
      return;
    }
    try {
      if (this.ready) {
        await this.request("shutdown", null);
      }
      this.notify("exit", {});
    } catch (_) {
      this.process.kill();
    }
  }

  dispose() {
    this.stop();
  }

  didOpen(document) {
    if (!this.ready || !isFlowArrowDocument(document)) {
      return;
    }
    this.notify("textDocument/didOpen", {
      textDocument: {
        uri: document.uri.toString(),
        languageId: "flowarrow",
        version: document.version,
        text: document.getText(),
      },
    });
  }

  didChange(document) {
    if (!this.ready || !isFlowArrowDocument(document)) {
      return;
    }
    this.notify("textDocument/didChange", {
      textDocument: {
        uri: document.uri.toString(),
        version: document.version,
      },
      contentChanges: [{ text: document.getText() }],
    });
  }

  didClose(document) {
    if (!this.ready || !isFlowArrowDocument(document)) {
      return;
    }
    this.notify("textDocument/didClose", {
      textDocument: { uri: document.uri.toString() },
    });
    this.diagnostics.delete(document.uri);
  }

  request(method, params) {
    if (!this.process || this.process.killed) {
      return Promise.resolve(null);
    }
    const id = this.nextId++;
    const message = { jsonrpc: "2.0", id, method, params };
    const promise = new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        if (this.pending.delete(id)) {
          reject(new Error(`FlowArrow LSP request timed out: ${method}`));
        }
      }, 5000);
      this.pending.set(id, { resolve, reject, timer });
    });
    this.send(message);
    return promise;
  }

  notify(method, params) {
    if (!this.process || this.process.killed) {
      return;
    }
    this.send({ jsonrpc: "2.0", method, params });
  }

  send(message) {
    const body = Buffer.from(JSON.stringify(message), "utf8");
    const header = Buffer.from(`Content-Length: ${body.length}\r\n\r\n`, "ascii");
    this.process.stdin.write(Buffer.concat([header, body]));
  }

  handleData(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    for (;;) {
      const headerEnd = this.buffer.indexOf("\r\n\r\n");
      if (headerEnd < 0) {
        return;
      }
      const header = this.buffer.slice(0, headerEnd).toString("ascii");
      const match = /Content-Length:\s*(\d+)/i.exec(header);
      if (!match) {
        this.buffer = this.buffer.slice(headerEnd + 4);
        continue;
      }
      const length = Number(match[1]);
      const bodyStart = headerEnd + 4;
      const bodyEnd = bodyStart + length;
      if (this.buffer.length < bodyEnd) {
        return;
      }
      const body = this.buffer.slice(bodyStart, bodyEnd).toString("utf8");
      this.buffer = this.buffer.slice(bodyEnd);
      try {
        this.handleMessage(JSON.parse(body));
      } catch (error) {
        this.output.appendLine(`Invalid FlowArrow LSP message: ${error.message}`);
      }
    }
  }

  handleMessage(message) {
    if (message.method === "textDocument/publishDiagnostics") {
      this.publishDiagnostics(message.params);
      return;
    }
    if (message.id !== undefined) {
      const pending = this.pending.get(message.id);
      if (!pending) {
        return;
      }
      this.pending.delete(message.id);
      clearTimeout(pending.timer);
      if (message.error) {
        pending.reject(new Error(message.error.message || "LSP request failed"));
      } else {
        pending.resolve(message.result);
      }
    }
  }

  publishDiagnostics(params) {
    if (!params?.uri) {
      return;
    }
    const uri = vscode.Uri.parse(params.uri);
    const items = (params.diagnostics || []).map((diagnostic) => {
      const item = new vscode.Diagnostic(
        toRange(diagnostic.range),
        diagnostic.message,
        toDiagnosticSeverity(diagnostic.severity),
      );
      item.source = diagnostic.source || "flowarrow";
      return item;
    });
    this.diagnostics.set(uri, items);
  }
}

class CompletionProvider {
  constructor(client) {
    this.client = client;
  }

  async provideCompletionItems(document, position) {
    const result = await this.client.request("textDocument/completion", {
      textDocument: { uri: document.uri.toString() },
      position: fromPosition(position),
    });
    return (result || []).map((item) => {
      const completion = new vscode.CompletionItem(item.label, toCompletionKind(item.kind));
      completion.detail = item.detail;
      return completion;
    });
  }
}

class DefinitionProvider {
  constructor(client) {
    this.client = client;
  }

  async provideDefinition(document, position) {
    const result = await this.client.request("textDocument/definition", {
      textDocument: { uri: document.uri.toString() },
      position: fromPosition(position),
    });
    if (!result) {
      return undefined;
    }
    return new vscode.Location(vscode.Uri.parse(result.uri), toRange(result.range));
  }
}

class HoverProvider {
  constructor(client) {
    this.client = client;
  }

  async provideHover(document, position) {
    const result = await this.client.request("textDocument/hover", {
      textDocument: { uri: document.uri.toString() },
      position: fromPosition(position),
    });
    if (!result?.contents) {
      return undefined;
    }
    const value =
      typeof result.contents === "string" ? result.contents : result.contents.value || "";
    return new vscode.Hover(new vscode.MarkdownString(value));
  }
}

class DocumentSymbolProvider {
  constructor(client) {
    this.client = client;
  }

  async provideDocumentSymbols(document) {
    const result = await this.client.request("textDocument/documentSymbol", {
      textDocument: { uri: document.uri.toString() },
    });
    return (result || []).map((symbol) => {
      return new vscode.DocumentSymbol(
        symbol.name,
        symbol.detail || "",
        toSymbolKind(symbol.kind),
        toRange(symbol.range),
        toRange(symbol.selectionRange),
      );
    });
  }
}

class FormattingProvider {
  constructor(context, output) {
    this.context = context;
    this.output = output;
  }

  async provideDocumentFormattingEdits(document) {
    if (!isFlowArrowDocument(document)) {
      return [];
    }

    let formatted;
    try {
      formatted = await formatDocument(this.context, document, this.output);
    } catch (error) {
      const message = error.message || String(error);
      this.output.appendLine(`FlowArrow format failed: ${message}`);
      vscode.window.showErrorMessage(`FlowArrow format failed: ${message}`);
      return [];
    }

    if (formatted === document.getText()) {
      return [];
    }
    return [vscode.TextEdit.replace(fullDocumentRange(document), formatted)];
  }
}

function resolveServerPath(context) {
  const configured = vscode.workspace.getConfiguration("flowarrow.server").get("path");
  if (configured && configured.trim()) {
    return configured;
  }

  const executable = process.platform === "win32" ? "flowarrow.exe" : "flowarrow";
  const candidates = [
    path.resolve(context.extensionPath, "..", "..", "target", "debug", executable),
    path.resolve(context.extensionPath, "..", "..", "target", "release", executable),
  ];
  for (const candidate of candidates) {
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }
  return executable;
}

function workspaceRoot() {
  return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
}

function workspaceRootUri() {
  return vscode.workspace.workspaceFolders?.[0]?.uri.toString() || null;
}

function isFlowArrowDocument(document) {
  return document.languageId === "flowarrow" && document.uri.scheme === "file";
}

async function formatDocument(context, document, output) {
  const command = resolveServerPath(context);
  output.appendLine(`Running ${command} fmt --stdin`);
  return await execFile(
    command,
    ["fmt", "--stdin"],
    { cwd: workspaceRoot() || context.extensionPath },
    document.getText(),
  );
}

function execFile(command, args, options, input) {
  return new Promise((resolve, reject) => {
    const child = cp.spawn(command, args, {
      ...options,
      stdio: ["pipe", "pipe", "pipe"],
    });
    const stdout = [];
    const stderr = [];
    child.stdout.on("data", (chunk) => stdout.push(chunk));
    child.stderr.on("data", (chunk) => stderr.push(chunk));
    child.on("error", reject);
    child.on("close", (code) => {
      const stdoutText = Buffer.concat(stdout).toString("utf8");
      const stderrText = Buffer.concat(stderr).toString("utf8").trim();
      if (code !== 0) {
        reject(new Error(stderrText || `flowarrow fmt exited with code ${code}`));
        return;
      }
      if (stderrText) {
        reject(new Error(stderrText));
        return;
      }
      resolve(stdoutText);
    });
    child.stdin.end(input);
  });
}

function fullDocumentRange(document) {
  const lastLine = document.lineAt(document.lineCount - 1);
  return new vscode.Range(0, 0, lastLine.range.end.line, lastLine.range.end.character);
}

function fromPosition(position) {
  return { line: position.line, character: position.character };
}

function toRange(range) {
  return new vscode.Range(
    range.start.line,
    range.start.character,
    range.end.line,
    range.end.character,
  );
}

function toDiagnosticSeverity(severity) {
  switch (severity) {
    case 1:
      return vscode.DiagnosticSeverity.Error;
    case 2:
      return vscode.DiagnosticSeverity.Warning;
    case 3:
      return vscode.DiagnosticSeverity.Information;
    default:
      return vscode.DiagnosticSeverity.Hint;
  }
}

function toCompletionKind(kind) {
  switch (kind) {
    case 3:
      return vscode.CompletionItemKind.Function;
    case 7:
      return vscode.CompletionItemKind.Struct;
    case 9:
      return vscode.CompletionItemKind.Module;
    case 14:
      return vscode.CompletionItemKind.Keyword;
    case 18:
      return vscode.CompletionItemKind.Reference;
    default:
      return vscode.CompletionItemKind.Variable;
  }
}

function toSymbolKind(kind) {
  switch (kind) {
    case 5:
      return vscode.SymbolKind.Struct;
    case 12:
      return vscode.SymbolKind.Function;
    default:
      return vscode.SymbolKind.Variable;
  }
}

module.exports = {
  activate,
  deactivate,
};
