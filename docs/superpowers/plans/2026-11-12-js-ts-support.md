# JavaScript and TypeScript Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `xray: Run` works in `.js` and `.ts` files with the same lenses and navigation as in Python.

**Architecture:** The Rust server parses JS/TS with `oxc`, locates the function and its call, blanks the non-setup top-level statements, inserts `__xray.*` probes into the function body and computes the static analysis. An embedded `tracer.js` run by `node -e` strips TS types, imports the instrumented code from a `data:` URL and ports `FrameRecorder`/`Recorder` from `tracer.py` to turn probe events into the existing trace JSON. Everything after `trace::Outcome` is unchanged.

**Tech Stack:** Rust 2024, `oxc_parser`/`oxc_ast`/`oxc_ast_visit`/`oxc_allocator`/`oxc_span` 0.158.0 (pinned with `=`), Node ≥ 23 (`module.registerHooks`, `module.stripTypeScriptTypes`), `tokio`, `serde`.

**Spec:** `docs/design.md`, section 12 (plus 9, 10, 11 for config, tests, scope).

## Global Constraints

- All repository text in English; no code comments.
- Lines and columns are 0-based everywhere in Rust and in the JSON.
- User-facing strings: `xray: async functions and generators are not supported`, `cannot start node: {error}`.
- Probe API names exactly: `__xray.enter(snap)`, `__xray.line(n, snap)`, `__xray.ret(value, snap)`, `__xray.raise(error)`, `__xray.exit()`.
- Values: `util.inspect(value, { breakLength: Infinity, depth: 4 })`, shortened to 60 chars with `…`.
- Python behavior and existing tests stay unchanged.

## Review Focus

- A file whose function body uses `let` declared *after* the probe point (TDZ) must not crash the run: values are just missing. Test in Task 3 (`tdz_declaration_later_in_block_does_not_fail`).
- Single-statement bodies without braces (`for (…) x++;`, `if (c) return a; else return b;`) must still produce per-line events. Test in Task 2 (`wraps_brace_less_bodies`) and Task 3 via `bubble.js`.
- A syntax error in an unrelated part of the file reports the parser message, not a crash. Test in Task 1.
- An arrow function with an expression body (`const sq = (x) => x * x`) traces its one line and return value. Test in Task 3 (`arrow_expression_body_records_return`).
- Two statements on one line (`a = 1; b = 2;`) merge into one event, as in Python. Test in Task 3 (`statements_on_one_line_merge`).

---

## File Structure

| File | Responsibility |
|---|---|
| `server/src/js/mod.rs` | `pub fn prepare(...) -> Prepared` entry point, `Prepared` enum |
| `server/src/js/locate.rs` | find the target function and the call statement, build the template |
| `server/src/js/edits.rs` | ordered text insertions/blanking applied to the source |
| `server/src/js/instrument.rs` | AST walk of the function body: probes, scope tracking for `snap`, analysis |
| `server/src/js/analysis.rs` | `Analysis` struct (serialized to `tracer.js`) |
| `server/src/tracer.js` | runtime: hooks, type stripping, probe API, tree building, JSON output |
| `server/src/tracer.rs` | `Language`, request dispatch, shared process execution |
| `server/src/config.rs`, `server.rs` | `node` option, language per document |
| `extension/extension.toml` | register JavaScript and TypeScript |
| `docs/samples/*.js, *.ts` | fixtures |

---

### Task 1: Locate the function and the call

**Files:**
- Modify: `server/Cargo.toml` (add the five `oxc_*` crates `= "0.158.0"`), `server/src/lib.rs` (`pub mod js;`)
- Create: `server/src/js/mod.rs`, `server/src/js/locate.rs`
- Create: `docs/samples/no_call.js`, `docs/samples/leetcode.js`
- Test: `server/tests/js_prepare.rs`

**Interfaces:**
- Produces:
  - `pub enum Prepared { Ready { code: String, analysis: Analysis, function: trace::Function }, NoFunction, NoCall(trace::Template), Failed(String) }` (`Ready` is filled in Task 2; in this task `prepare` returns `Ready` with `code` = the unmodified source and `Analysis::default()`).
  - `pub fn prepare(source: &str, typescript: bool, line: u32, name: Option<&str>) -> Prepared`
  - Parse with `SourceType::ts()` or `SourceType::mjs()`, always `.with_module(true)`.

- [ ] **Step 1: Write failing tests**
  - `finds_a_var_function_expression_like_leetcode`: `leetcode.js` = `var twoSum = function(nums, target) { … };` + `console.log(twoSum([2, 7, 11, 15], 9));`, cursor inside the body → `Ready` with `function.name == "twoSum"`, `function.line`/`end_line` = the declaration's lines.
  - `function_without_a_call_yields_a_template`: `no_call.js` = `function add(a, b) {\n  return a + b;\n}\n` at line 1 → `NoCall(Template { after_line: 2, text: "add(/* a */, /* b */);" })`.
  - `cursor_outside_a_function_yields_no_function` → `NoFunction`.
  - `falls_back_to_the_name_when_the_cursor_moved`: line outside, `name = Some("add")` → not `NoFunction`.
  - `destructured_parameter_gets_a_positional_placeholder`: `function f({a}, b) {}` → template `f(/* arg1 */, /* b */);`.
  - `async_function_is_rejected`: `async function f() {}` + call → `Failed("async functions and generators are not supported")`.
  - `syntax_error_reports_the_parser_message_and_line`: `function f( {\n` → `Failed(m)` where `m` starts with `line 1: ` (1-based for humans).
  - `typescript_annotations_parse`: `function f(n: number): number { return n }` with `typescript = true`, + call → `Ready`.
- [ ] **Step 2:** `cargo test --test js_prepare` → FAIL (`js` module missing).
- [ ] **Step 3:** Implement `locate.rs`: top-level `FunctionDeclaration`, `ExportNamedDeclaration` wrapping one, and `VariableDeclaration` declarators whose `init` is `FunctionExpression`/`ArrowFunctionExpression`; range = the whole statement span converted to lines. Call = first top-level statement other than the function's own, containing a `CallExpression` whose callee is `IdentifierReference` named like the function (use `oxc_ast_visit::Visit`). The `Failed` message for async/generator is the spec string without the `xray: ` prefix (the view adds it).
- [ ] **Step 4:** `cargo test` → all pass.
- [ ] **Step 5:** Commit `Locate JS/TS functions and their calls`.

### Task 2: Instrumentation and analysis

**Files:**
- Create: `server/src/js/edits.rs`, `server/src/js/instrument.rs`, `server/src/js/analysis.rs`
- Modify: `server/src/js/mod.rs`
- Test: `server/tests/js_prepare.rs`

**Interfaces:**
- Consumes: Task 1's `prepare`, located function and call.
- Produces:
  - `#[derive(Debug, Default, Clone, PartialEq, Serialize)] pub struct Analysis { pub loops: BTreeMap<u32, LoopInfo>, pub assigned: BTreeMap<u32, BTreeSet<String>>, pub conditions: BTreeMap<u32, (u32, u32)> }`
  - `#[derive(... Serialize)] pub struct LoopInfo { pub line: u32, pub end_line: u32, pub body_start: u32, pub body_end: u32, pub targets: Option<Vec<String>> }` — same keys as `loops_of` in `tracer.py`.
  - `Prepared::Ready.code`: the instrumented module source.
  - `edits.rs`: `Edits::insert(offset: u32, text: String)`, `Edits::blank(span: Span)` (non-newline chars → spaces), `Edits::apply(&self, source: &str) -> String`; insertions at the same offset keep insertion order.

- [ ] **Step 1: Write failing tests** (assert on substrings of `code` and on `analysis`)
  - `blanks_non_setup_statements_and_appends_the_call`: `const K = 2;\nconsole.log("x");\nfunction f() { return K }\nf();\n` → line 1 of `code` is all spaces, `code` ends with `\nf();\n`, line 0 unchanged, line count of the original part unchanged.
  - `probes_each_statement_with_variables_declared_before_it`: `function f(a) {\n  let b = a;\n  b++;\n}` → contains `__xray.line(1, () => ({a}));` before `let b` and `__xray.line(2, () => ({a, b}));` before `b++`.
  - `wraps_tests_of_if_while_and_for`: `if ((__xray.line(N, () => ({…})), c))` for `if`, `while`, `for (let i = 0; (__xray.line(N, () => ({…, i})), i < n); i++)`; `for (;;)` gets `(__xray.line(N, …), true)`.
  - `wraps_brace_less_bodies`: `for (const x of xs) s += x;` → `{ __xray.line(N, …); __xray.line(M, …); s += x; }` shape: the body is in braces and the header probe is first in it.
  - `wraps_the_body_and_returns`: body starts with `__xray.enter(() => ({a})); try {`, `return a + 1;` → `return __xray.ret(a + 1, () => ({a}));`, bare `return;` → `return __xray.ret(undefined, …);`, body ends with `__xray.ret(undefined, …); } catch (e) { __xray.raise(e); throw e } finally { __xray.exit() }`.
  - `arrow_expression_body_becomes_a_block`: `const sq = (x) => x * x;` → body is `{ __xray.enter(…); try { __xray.line(0, …); return __xray.ret(x * x, …); } catch … }`.
  - `does_not_instrument_nested_functions`: inner `function g() { let z = 1 }` and `xs.map(v => v + 1)` contain no `__xray`.
  - `analysis_matches_the_python_shape`: for `bubble.js` (Task 3 fixture, add it now): loops at the two `for` header lines with `targets: Some(["i"])`/`Some(["j"])`, `body_start`/`body_end` = first/last body statement lines; `conditions[if_line] == (first_body_line, last_body_line)`; `assigned` contains `i` on the outer header line and the swapped names on the swap line.
- [ ] **Step 2:** `cargo test --test js_prepare` → new tests FAIL.
- [ ] **Step 3:** Implement `instrument.rs` as a recursive walk over the function's statements (not `Visit`; it needs control over scope and order). Keep a stack of scopes `Vec<Vec<(String, u32 /*decl start*/)>>`: push for the function (params), each block, each `for` head, each `catch`; `var` and nested `function` names go to the function scope. `snap` at offset `p` = `() => ({` + names from all scopes with decl start `< p`, in declaration order, deduplicated (inner wins) + `})`. Probe rules: every statement gets `__xray.line(line, snap);` before it, except `if`/`while`/`for(;;)` (test wrapped) and `do…while` (probe first in body with the `do` line). `for…of`/`for…in` also get a probe first in the body with the header line. Brace-less bodies get `{` / `}` around them before the inner probe is inserted. Skip `EmptyStatement`. Do not descend into function/arrow/class nodes. Lines come from a precomputed table of line start offsets (binary search).
- [ ] **Step 4:** `cargo test` → all pass.
- [ ] **Step 5:** Commit `Instrument JS/TS functions with probes`.

### Task 3: tracer.js and running Node

**Files:**
- Create: `server/src/tracer.js`
- Modify: `server/src/tracer.rs`, `server/tests/common/mod.rs` (`trace_sample` picks language from the file extension, `program` = `python3` or `node`)
- Create samples: `bubble.js`, `nested_while.js`, `fib.ts`, `raises.js`, `forever.js`, `helpers.ts`, `helper_module.ts` (exports a constant imported by `helpers.ts`), `one_line.js` (contains `a = 1; b = 2;`, a TDZ case and `const sq = (x) => x * x`) — port the `.py` samples line-for-line where one exists, so expected values mirror the Python tests
- Test: `server/tests/js_tracer.rs`

**Interfaces:**
- Consumes: `js::prepare`, `Analysis` (Task 2).
- Produces:
  - `pub enum Language { Python, JavaScript, TypeScript }` with `pub fn from_id(id: &str) -> Option<Language>` (`"python"`, `"javascript"`, `"typescript"`).
  - `TraceRequest { language: Language, program: PathBuf, path, source, line, name, limits }` (`python` renamed to `program`).
  - `tracer::run` dispatches; a private `execute(command: Command, input: String, limits: &Limits) -> Outcome` holds the existing spawn/timeout/read-output logic for both languages.
  - Node is started as `node -e <tracer.js> <path> <output> <timeout_ms> <max_steps>` with stdin JSON `{ "code": …, "typescript": bool, "analysis": Analysis, "function": { "name", "line", "end_line" } }`. `Prepared::NoFunction`/`NoCall`/`Failed` map straight to `Outcome` without starting Node.
  - `tracer.js` output: the same JSON object as `tracer.py`'s `run` (`status`, `function`, `loops`, `calls`, `stopped`).

- [ ] **Step 1: Write failing tests** mirroring `tests/tracer.rs` on JS fixtures:
  - `js_records_changed_values_and_the_return_value_per_line` (`helpers.ts`, includes a value from `helper_module.ts` and a `node:` import)
  - `js_splits_loops_into_iterations_scoped_to_the_outer_iteration` (`nested_while.js`, labels `i=0`, `i=1`, `i=2`)
  - `js_infers_conditions` (`bubble.js`, `→ true/false` on the `if` line)
  - `js_records_in_place_mutation` (`bubble.js`, `arr = [ 1, 3, 2 ]`-style inspect output on the swap line)
  - `ts_recursion_has_calls_with_depth` (`fib.ts`)
  - `js_exception_is_recorded_once_at_the_raising_line` (`raises.js`, `💥` text is `TypeError: …` in the `exception` field, same form as Python's `Type: message`)
  - `js_step_limit_stops_and_keeps_data` / `js_timeout_stops` (`forever.js`, same limits as the Python tests)
  - `statements_on_one_line_merge`, `tdz_declaration_later_in_block_does_not_fail`, `arrow_expression_body_records_return` (`one_line.js`)
  - `missing_node_reports_cannot_start` (`program = "/nonexistent/node"`) → `Failed` starting with `cannot start node:`
- [ ] **Step 2:** `cargo test --test js_tracer` → FAIL.
- [ ] **Step 3:** Implement `tracer.js`: read stdin JSON, `registerHooks` resolve hook mapping a `data:` parent to the real file URL, set `globalThis.__xray`, `require` (`createRequire(path)`), `__filename`, `__dirname`, strip types if `typescript`, `await import("data:text/javascript," + encodeURIComponent(code))`. Port `FrameRecorder`/`Recorder` from `tracer.py`; differences: a snapshot is the object returned by `snap()` (inside `try`; on throw `{}`), values merge into the previous snapshot instead of replacing it, `enter` pushes a recorder (depth = stack length − 1), `exit` pops it, `ret` finishes the pending event with the snapshot and sets `return`, `raise` sets `exception` once per error object and ignores the stop sentinel. `line` counts steps and checks `performance.now()` against the deadline, throwing the stop sentinel. Rejection of the import: stop sentinel → `stopped`; any other error with no recorded call → `failed` with `Type: message`. Write the JSON with `fs.writeFileSync(output)`.
- [ ] **Step 4:** `cargo test` → all pass (Python tests too).
- [ ] **Step 5:** Commit `Trace JS/TS functions with Node`.

### Task 4: Server and extension wiring

**Files:**
- Modify: `server/src/config.rs` (`node: Option<PathBuf>`), `server/src/server.rs` (`Document.language`, `node()` = config or `"node"`, `start_run` builds `TraceRequest` per language), `extension/extension.toml` (`languages = ["Python", "JavaScript", "TypeScript"]`, description mentions JS/TS), `server/tests/common/lsp.rs` (`open` sends `languageId` from the extension: `py`→`python`, `js`→`javascript`, `ts`→`typescript`), `docs/install-dev.md` (Node ≥ 23 prerequisite, `node` option)
- Test: `server/tests/lsp.rs`

**Interfaces:**
- Consumes: `Language::from_id`, `TraceRequest` (Task 3).

- [ ] **Step 1: Write failing tests**
  - `running_a_javascript_file_shows_values_below_the_lines`: open `bubble.js` with `languageId: javascript`, `run_at(function line)`, lenses contain a value lens under the first body line.
  - `a_typescript_function_without_a_call_gets_a_template_inserted`: `no_call.ts` → `workspace/applyEdit` with `add(/* a */, /* b */);`.
  - `config_reads_the_node_option` (unit in `config.rs` style of existing tests if any, else in `lsp.rs` via a bad `node` path → warning lens `⚠ xray: cannot start node: …`).
- [ ] **Step 2:** `cargo test --test lsp` → FAIL.
- [ ] **Step 3:** Implement the wiring; `Document::language` defaults to `Python` (`#[derive(Default)]`), set from `languageId` in `did_open`.
- [ ] **Step 4:** Full verification: `cargo test`, `cargo clippy --all-targets`, `cargo build --release` in `server/`, `cargo build --release --target wasm32-wasip2` in `extension/`.
- [ ] **Step 5:** Commit `Run xray in JavaScript and TypeScript files`.
