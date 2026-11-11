# xray — design

xray is a Zed extension that shows what happens inside one Python function, line by line and loop iteration by loop iteration. It is meant for understanding algorithmic code: which values each line produces, and how they evolve across iterations of (nested) loops and recursive calls.

Zed behavior referenced here (code lens rendering, refresh, click handling) was researched for tldc; see `~/Work/projects/tldc/docs/zed-research.md`.

## 1. Scope

- Language: **Python 3.12+** only.
- One function at a time, the one containing the cursor.
- Everything user-facing (lens titles, menu entries, messages) is in English.

## 2. Repository layout

Same layout as tldc. No root Cargo workspace; the two crates build independently.

| Path | Content |
|---|---|
| `server/` | Rust crate, binary `xray`, LSP over stdio |
| `server/src/tracer.py` | Python tracer, embedded in the binary with `include_str!` |
| `extension/` | Zed extension crate (cdylib, `wasm32-wasip2`), `zed_extension_api` 0.7.0 |
| `docs/` | design, install notes |
| `docs/samples/` | Python fixtures used by the tests |

Extension id, language server id and name: `xray`. The language server is registered for the `Python` language.

## 3. Triggering a run

Entry point is the code action menu (Ctrl+.). Zed cannot bind a key to a specific code action.

### 3.1 Code actions

| Title | Offered when | Command |
|---|---|---|
| `xray: Run` | always (the tracer decides whether the cursor is inside a function) | `xray.run` |
| `xray: Clear` | the document has an active trace | `xray.clear` |

### 3.2 Finding the function and the call

1. The tracer finds the **top-level** `def` enclosing the cursor position (decorator lines count). Methods and nested functions are not supported, because they cannot be called from a top-level statement. Without one the server shows the message `xray: put the cursor inside a top-level function`.
2. It looks for the first **top-level** statement in the file that calls that function by name, `name(...)`.
3. If there is no such call, the server inserts a call template after the function, separated by two blank lines, using the parameter names, e.g. `bubble_sort(arr=, k=)`, via `workspace/applyEdit`, and moves the cursor inside the parentheses (`window/showDocument` with a selection). Nothing runs. The user fills in the arguments, saves, and runs again.

The inserted call stays in the file as ordinary code.

### 3.3 What is executed

The tracer builds a module from the file's top-level statements, keeping only:

- `import` and `from … import`
- `def` and `class`
- simple assignments (`NAME = …`, including annotated assignments)

It skips bare expressions and calls (`main()`, `print(…)`), `if __name__ == "__main__":` blocks, every other statement, and the found call statement itself. Then it executes the found call statement under the tracer. The module directory is put on `sys.path` so local imports work. A decorated function is unwrapped (`inspect.unwrap`) so the original code is traced.

### 3.4 Interpreter

First match wins:

1. `initialization_options.python`
2. `.venv/bin/python` in the worktree root
3. `$VIRTUAL_ENV/bin/python`
4. `python3` on `PATH`

### 3.5 Tracer process

- Started as `<python> -I -c <tracer source> <args>`; no Python package is installed.
- Arguments: the file path, the cursor line, the function name (empty on a first run, used on re-runs when the `def` line was edited), the output file path, the timeout in ms, the step limit. The document text (including unsaved changes) comes on stdin.
- The tracer writes its result as JSON to the output file (a temp file owned by the server).
- The user code's stdout and stderr are discarded. A traceback of the tracer itself is written to the LSP log.

## 4. Tracing

Based on `sys.settrace`.

### 4.1 What is traced

- Line events in frames of the selected function, including recursive calls of it.
- Not traced: calls into other functions (helpers, libraries), nested `def` and `lambda`. Their effect is visible through the variables they assign to on the calling line.
- Comprehensions are inlined in 3.12+ and count as a single line: the result is shown, not their inner iterations. Generator expressions run in their own frame and are not traced.

### 4.2 What is recorded per executed line

- The names the line **assigns to** (statically, from the AST: assignment, augmented and annotated assignment, walrus and `for` targets), always, even if the value did not change (`total += 0`).
- Every other variable whose value **changed**: new bindings and in-place mutations. After each line the tracer stores the `repr` of every local and compares it with the previous one. Comparing `repr`s instead of deep copies avoids copying cost and objects without `__eq__`; the trade-off is that a mutation invisible in `repr` (a plain object with the default `repr`) is not detected.
- Repeated line events of the same line in a row (an inlined comprehension) are merged into one event.
- For `return`: the return value.
- For `if`, `elif` and `while` headers: the condition outcome, **inferred** from the next executed line (entering the body = `True`, anything else = `False`). The condition is never re-evaluated.
- For an exception: the exception type and message at the line where it was raised. Values recorded up to that point are kept.

Values are stored after the line has executed.

### 4.3 Structure of the trace

The tracer requires Python 3.12+ and reports a failure otherwise.

The trace is a tree of frames:

- **call**: one execution of the selected function. Recursive calls are children of the call that made them, with a depth.
- **loop**: one `for` or `while` statement inside a call. A loop has an ordered list of **iterations**.
- **iteration**: the line events inside one pass of the loop body, plus the nested loops started in that pass. A `for` iteration carries a label with its target values, e.g. `k=0, item='a'`.

The header event that ends a loop (iterator exhausted, `while` condition false) belongs to the enclosing scope. A loop that never iterates has no iterations and gets no navigation.

Inner loops are scoped to one iteration of the outer loop. A loop that runs again in a later outer iteration is a new set of iterations.

### 4.4 Limits

| Limit | Default | Setting |
|---|---|---|
| Wall time | 5 s | `timeoutMs` |
| Recorded line events | 100 000 | `maxSteps` |

When a limit is hit, the tracer stops and the server shows whatever was recorded so far, plus a warning lens (6.3).

## 5. View state

The server keeps per document:

- the active trace (function, call source, trace tree),
- the selected **call** (default: the first, top-level call),
- the selected **iteration** of every loop visible in the selected call (default: the first).

Changing the selected call resets every loop cursor to its first iteration. Changing the selected iteration of a loop resets the cursors of the loops nested in it.

What a line shows is resolved through the current cursors: the events of that line in the selected call, inside the selected iteration of each enclosing loop. If a line executed more than once in that scope (typically a `while` header), the last event wins. A line that did not execute in that scope shows nothing.

Lines after a loop show their values from after the loop finished.

## 6. Code lenses

Zed renders a lens as a block **above** its line. All lenses on the same row, from all servers, are joined with ` | `, and each part is clickable on its own.

### 6.1 Values below the line

The values of line N are emitted as a lens on line N+1, so they read as being below line N. Exception: when line N is the last line of the file, the lens is on line N.

Title format, parts joined by two spaces:

| Event | Part |
|---|---|
| changed variable | `name = repr` |
| return | `↩ repr` |
| condition | `→ True` / `→ False` |
| exception | `💥 Type: message` |

Each `repr` is truncated to 60 chars with `…`. Parts are wrapped at part boundaries (never inside a value) so no line exceeds 80 chars, unless a single part is longer.

On a row shared by values and navigation, values come first.

Value lenses carry a no-op command (`xray.noop`), because Zed does not render lenses without a command.

### 6.2 Navigation

| Where | Parts | Commands |
|---|---|---|
| above a `for`/`while` header | `◀` · `{var}={value} · {k}/{n}` · `▶` | `xray.prevIteration`, `xray.noop`, `xray.nextIteration` |
| above the `def` | `◀` · `call {k}/{n} · depth {d}` · `▶` · `✕` | `xray.prevCall`, `xray.noop`, `xray.nextCall`, `xray.clear` |

`{var}={value}` is the iteration label (4.3); `while` loops have none and show only `{k}/{n}`. `◀`/`▶` stop at the ends; they do not wrap. The call navigation is shown only when there is more than one call.

A navigation lens and a value lens can land on the same row; Zed joins them with ` | `, which is acceptable.

### 6.3 Status lenses above the `def`

| State | Title |
|---|---|
| running (first run only, on the cursor line) | `⏳ xray: running…` |
| step limit | `⚠ xray: stopped after {n} steps` |
| timeout | `⚠ xray: stopped after {s} s` |
| tracer failure (replaces all other lenses) | `⚠ xray: {message}` |

Status lenses run `xray.clear` on click. If the tracer process outlives the timeout by 1 s (an endless loop in untraced code), it is killed and reported as a failure without data.

## 7. Commands

All arguments are positional. `executeCommand` returns `null` immediately; the result is surfaced through `workspace/codeLens/refresh`. Every command is listed in `executeCommandProvider.commands`, otherwise Zed drops the click silently. A lens click clears the editor selection, so no command depends on it.

| Command | Arguments | Effect |
|---|---|---|
| `xray.run` | `[uri, position]` | Find function and call (3.2), run the tracer, store the trace |
| `xray.clear` | `[uri]` | Drop the trace and its lenses |
| `xray.prevIteration` / `xray.nextIteration` | `[uri, loopId]` | Move one loop cursor |
| `xray.prevCall` / `xray.nextCall` | `[uri]` | Move the call cursor |
| `xray.noop` | `[]` | Nothing |

`loopId` is an opaque server-generated id, unique within the current trace.

## 8. Lifecycle

- **Edit** (`didChange`): lenses shift with the line delta. Lines touched by an edit lose their values until the next run.
- **Save** (`didSave`): if the document has an active trace, the same function is run again with the call found again by 3.2. The old values stay visible until the new trace arrives. Loop and call cursors are kept and clamped to the new trace. Other functions in the file are never run.
- **Clear**: `xray.clear` from the menu or the `✕` lens.
- State lives in memory keyed by `uri`.

## 9. Configuration (`initializationOptions`)

```json
{
  "python": null,
  "timeoutMs": 5000,
  "maxSteps": 100000
}
```

Every field is optional.

Zed needs `"code_lens": "on"` in the user settings, or no lens is rendered.

## 10. Testing

- TDD with `cargo test` in `server/`. No separate Python test runner.
- Tests run the real tracer with `python3` against the fixtures in `docs/samples/`:
  - bubble sort (in-place mutation, nested loops, conditions)
  - nested loops with an inner `while`
  - recursive Fibonacci (call navigation, depth)
  - a function raising an exception
  - an infinite loop (step limit and timeout)
  - a function using a helper and a module constant from the same file, plus a stdlib import
- Tests go through the server's public interface: run a trace, move cursors, read the lenses.

## 11. Out of scope for v1

- Languages other than Python.
- A keybinding for running (would need a Zed task plus a side channel into the server).
- Live re-run while typing.
- Showing `print` output.
- Highlighting changed elements inside collections.
- Jump to first/last iteration.
- Operand values of conditions (would need re-evaluation).
