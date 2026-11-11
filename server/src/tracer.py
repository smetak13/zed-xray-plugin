import ast
import inspect
import io
import json
import os
import sys
import time

MAX_REPR = 60
MINIMUM_PYTHON = (3, 12)
SETUP_STATEMENTS = (ast.Import, ast.ImportFrom, ast.FunctionDef, ast.ClassDef, ast.Assign, ast.AnnAssign)


def top_level_functions(tree):
    return [node for node in tree.body if isinstance(node, ast.FunctionDef)]


def find_function(tree, line, name):
    functions = top_level_functions(tree)
    for node in functions:
        start = min([node.lineno] + [decorator.lineno for decorator in node.decorator_list]) - 1
        if start <= line <= node.end_lineno - 1:
            return node
    for node in functions:
        if node.name == name:
            return node
    return None


def calls_function(statement, name):
    if isinstance(statement, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
        return False
    return any(
        isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id == name
        for node in ast.walk(statement)
    )


def find_call(tree, name):
    return next((statement for statement in tree.body if calls_function(statement, name)), None)


def call_template(function):
    arguments = function.args
    names = [argument.arg for argument in arguments.posonlyargs + arguments.args + arguments.kwonlyargs]
    return f"{function.name}({', '.join(name + '=' for name in names)})"


class Stopped(BaseException):
    def __init__(self, reason):
        self.reason = reason


def full_repr(value):
    try:
        return repr(value)
    except Exception as error:
        return f"<repr failed: {type(error).__name__}>"


def shorten(text):
    return text if len(text) <= MAX_REPR else text[: MAX_REPR - 1] + "…"


def snapshot(frame):
    return {name: full_repr(value) for name, value in frame.f_locals.items()}


NESTED_SCOPES = (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda, ast.ClassDef)


def own_nodes(function):
    stack = list(function.body)
    while stack:
        node = stack.pop()
        yield node
        stack.extend(child for child in ast.iter_child_nodes(node) if not isinstance(child, NESTED_SCOPES))


def target_names(target):
    return [node.id for node in ast.walk(target) if isinstance(node, ast.Name) and isinstance(node.ctx, ast.Store)]


def condition_bodies(function):
    return {
        node.lineno - 1: (node.body[0].lineno - 1, node.body[-1].end_lineno - 1)
        for node in own_nodes(function)
        if isinstance(node, (ast.If, ast.While)) and node.body[0].lineno != node.lineno
    }


def loops_of(function):
    loops = {}
    for node in own_nodes(function):
        if isinstance(node, (ast.For, ast.While)):
            loops[node.lineno - 1] = {
                "line": node.lineno - 1,
                "end_line": node.end_lineno - 1,
                "body_start": node.body[0].lineno - 1,
                "body_end": node.body[-1].end_lineno - 1,
                "targets": target_names(node.target) if isinstance(node, ast.For) else None,
            }
    return loops


def assigned_names(function):
    assigned = {}
    for node in own_nodes(function):
        if isinstance(node, ast.Assign):
            targets = node.targets
        elif isinstance(node, (ast.AugAssign, ast.AnnAssign, ast.NamedExpr, ast.For)):
            targets = [node.target]
        else:
            continue
        names = assigned.setdefault(node.lineno - 1, set())
        for target in targets:
            names.update(target_names(target))
    return assigned


def new_scope(label=None):
    return {"label": label, "events": [], "loops": []}


class Analysis:
    def __init__(self, function):
        self.loops = loops_of(function)
        self.assigned = assigned_names(function)
        self.conditions = condition_bodies(function)


class ActiveLoop:
    def __init__(self, info, run):
        self.info = info
        self.run = run

    def contains(self, line):
        return line == self.info["line"] or self.info["body_start"] <= line <= self.info["body_end"]

    def scope(self):
        return self.run["iterations"][-1]


class FrameRecorder:
    def __init__(self, owner, call, analysis):
        self.owner = owner
        self.call = call
        self.loops = analysis.loops
        self.assigned = analysis.assigned
        self.conditions = analysis.conditions
        self.active = []
        self.pending = None
        self.labelled = None
        self.values = None
        self.unwinding = False

    def scope(self):
        return self.active[-1].scope() if self.active else self.call["scope"]

    def leave_loops(self, line):
        while self.active and (line is None or not self.active[-1].contains(line)):
            loop = self.active.pop()
            iterations = loop.run["iterations"]
            last = iterations[-1]
            if len(last["events"]) == 1 and not last["loops"]:
                iterations.pop()
                self.scope()["events"].append(last["events"][0])
            if not iterations:
                self.scope()["loops"].remove(loop.run)

    def enter_line(self, line):
        self.leave_loops(line)
        info = self.loops.get(line)
        if info is not None:
            if not (self.active and self.active[-1].info is info):
                run = {"line": line, "iterations": []}
                self.scope()["loops"].append(run)
                self.active.append(ActiveLoop(info, run))
            self.active[-1].run["iterations"].append(new_scope())
            self.labelled = self.active[-1] if info["targets"] is not None else None
        else:
            self.labelled = None
        self.pending = {"line": line, "values": []}
        self.scope()["events"].append(self.pending)

    def start(self, frame):
        self.values = snapshot(frame)

    def finish_pending(self, frame, next_line):
        values = snapshot(frame)
        body = self.conditions.get(self.pending["line"]) if self.pending is not None else None
        if body is not None and next_line is not None:
            self.pending["condition"] = body[0] <= next_line <= body[1]
        if self.pending is not None:
            assigned = self.assigned.get(self.pending["line"], ())
            self.pending["values"] = [
                [name, shorten(text)]
                for name, text in values.items()
                if name in assigned or self.values.get(name) != text
            ]
        if self.labelled is not None:
            names = self.labelled.info["targets"]
            self.labelled.scope()["label"] = ", ".join(
                f"{name}={shorten(values[name])}" for name in names if name in values
            )
            self.labelled = None
        self.values = values

    def trace(self, frame, event, argument):
        if event == "line":
            if self.pending is not None and self.pending["line"] == frame.f_lineno - 1:
                return self.trace
            self.owner.count_step()
            self.unwinding = False
            self.finish_pending(frame, frame.f_lineno - 1)
            self.enter_line(frame.f_lineno - 1)
        elif event == "return":
            self.finish_pending(frame, None)
            self.leave_loops(None)
            self.owner.depth -= 1
            if self.pending is not None and not self.unwinding:
                self.pending["return"] = shorten(full_repr(argument))
        elif event == "exception":
            self.unwinding = True
            error = argument[1]
            if self.pending is not None and not any(error is seen for seen in self.owner.exceptions):
                self.owner.exceptions.append(error)
                self.pending["exception"] = error_message(error)
        return self.trace


class Recorder:
    def __init__(self, code, analysis, deadline, max_steps):
        self.code = code
        self.analysis = analysis
        self.deadline = deadline
        self.max_steps = max_steps
        self.steps = 0
        self.stopped = None
        self.calls = []
        self.depth = 0
        self.exceptions = []

    def count_step(self):
        self.steps += 1
        if self.steps > self.max_steps:
            raise Stopped("steps")
        if time.monotonic() > self.deadline:
            raise Stopped("timeout")

    def trace(self, frame, event, argument):
        if frame.f_code is not self.code:
            return None
        call = {"depth": self.depth, "scope": new_scope()}
        self.calls.append(call)
        recorder = FrameRecorder(self, call, self.analysis)
        self.depth += 1
        recorder.start(frame)
        return recorder.trace


def build_namespace(path, tree, call):
    setup = [statement for statement in tree.body if isinstance(statement, SETUP_STATEMENTS) and statement is not call]
    namespace = {"__name__": "__xray__", "__file__": path}
    exec(compile(ast.Module(body=setup, type_ignores=[]), path, "exec"), namespace)
    return namespace


def error_message(error):
    return f"{type(error).__name__}: {error}"


def main():
    path, line, name, output = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4]
    deadline = time.monotonic() + int(sys.argv[5]) / 1000
    max_steps = int(sys.argv[6])
    source = sys.stdin.read()
    result = run(path, source, line, name, deadline, max_steps)
    with open(output, "w") as file:
        json.dump(result, file)


def run(path, source, line, name, deadline, max_steps):
    if sys.version_info < MINIMUM_PYTHON:
        return {"status": "failed", "message": f"Python 3.12+ required, found {sys.version.split()[0]}"}
    tree = ast.parse(source, path)
    function = find_function(tree, line, name)
    if function is None:
        return {"status": "no_function"}
    call = find_call(tree, function.name)
    if call is None:
        return {
            "status": "no_call",
            "template": {"after_line": function.end_lineno - 1, "text": call_template(function)},
        }
    sys.path.insert(0, os.path.dirname(os.path.abspath(path)))
    real_stdout, real_stderr = sys.stdout, sys.stderr
    sys.stdout = sys.stderr = io.StringIO()
    try:
        namespace = build_namespace(path, tree, call)
        target = inspect.unwrap(namespace[function.name])
        analysis = Analysis(function)
        recorder = Recorder(target.__code__, analysis, deadline, max_steps)
        call_code = compile(ast.Module(body=[call], type_ignores=[]), path, "exec")
        sys.settrace(recorder.trace)
        try:
            exec(call_code, namespace)
        except Stopped as stop:
            recorder.stopped = stop.reason
        except Exception as error:
            if not recorder.calls:
                return {"status": "failed", "message": error_message(error)}
        finally:
            sys.settrace(None)
    except Exception as error:
        return {"status": "failed", "message": error_message(error)}
    finally:
        sys.stdout, sys.stderr = real_stdout, real_stderr
    return {
        "status": "traced",
        "function": {"name": function.name, "line": function.lineno - 1, "end_line": function.end_lineno - 1},
        "loops": [{"line": info["line"], "end_line": info["end_line"]} for info in sorted(analysis.loops.values(), key=lambda info: info["line"])],
        "calls": recorder.calls,
        "stopped": recorder.stopped,
    }


main()
