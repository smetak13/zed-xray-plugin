# xray — dev install (Linux, Omarchy)

Prerequisites: Rust with the `wasm32-wasip2` target, Python 3.12+, Node 23+ (for JavaScript and TypeScript), Zed.

## 1. Build the server

```sh
cd ~/Work/projects/xray/server
cargo build --release
```

## 2. Configure Zed

Open settings (`zed: open settings`) and add:

```json
{
  "code_lens": "on",
  "lsp": {
    "xray": {
      "binary": {
        "path": "/home/simon/Work/projects/xray/server/target/release/xray"
      },
      "initialization_options": {
        "timeoutMs": 5000,
        "maxSteps": 100000
      }
    }
  }
}
```

- `"code_lens": "on"` is required, otherwise no lens is rendered.
- `initialization_options.node` overrides the Node binary used for JavaScript and TypeScript files (default `node` on `PATH`).
- `initialization_options.python` overrides the interpreter. Without it xray uses `.venv/bin/python` in the worktree root, then `$VIRTUAL_ENV/bin/python`, then `python3`.

## 3. Install the extension

Start Zed from a shell so it finds `cargo`/`rustc`, then run `zed: install dev extension` and pick `~/Work/projects/xray/extension`.

## 4. Use it

1. Put the cursor inside a top-level function.
2. Ctrl+. → `xray: Run`.
3. Without a top-level call of the function, xray inserts `name(arg=, …)` below it. Fill in the arguments and run again.
4. Click `◀` / `▶` above a loop or above the `def` (recursion) to move between iterations and calls.
5. Save to run again. `✕` or Ctrl+. → `xray: Clear` removes everything.

## Logs

`dev: open language server logs` → `xray`. Tracer crashes end up there as the last stderr line.

## Tests

```sh
cd ~/Work/projects/xray/server
cargo test
```
