const fs = require("node:fs");
const path = require("node:path");
const { inspect } = require("node:util");
const { createRequire, registerHooks, stripTypeScriptTypes } = require("node:module");
const { pathToFileURL } = require("node:url");

const MAX_REPR = 60;
const [file, output, timeoutMs, maxSteps] = process.argv.slice(1);

class Stopped {
  constructor(reason) {
    this.reason = reason;
  }
}

function fullRepr(value) {
  try {
    return inspect(value, { breakLength: Infinity, depth: 4 });
  } catch (error) {
    return `<repr failed: ${error?.name}>`;
  }
}

function shorten(text) {
  const characters = [...text];
  return characters.length <= MAX_REPR ? text : characters.slice(0, MAX_REPR - 1).join("") + "…";
}

function snapshot(snap) {
  try {
    return Object.fromEntries(Object.entries(snap()).map(([name, value]) => [name, fullRepr(value)]));
  } catch {
    return null;
  }
}

function errorMessage(error) {
  return error instanceof Error ? `${error.name}: ${error.message}` : fullRepr(error);
}

function newScope(label = null) {
  return { label, events: [], loops: [] };
}

class ActiveLoop {
  constructor(info, run) {
    this.info = info;
    this.run = run;
  }

  contains(line) {
    return line === this.info.line || (this.info.body_start <= line && line <= this.info.body_end);
  }

  scope() {
    return this.run.iterations.at(-1);
  }
}

class FrameRecorder {
  constructor(analysis, call, values) {
    this.loops = analysis.loops;
    this.assigned = analysis.assigned;
    this.conditions = analysis.conditions;
    this.call = call;
    this.values = values ?? {};
    this.active = [];
    this.pending = null;
    this.labelled = null;
    this.finished = false;
  }

  scope() {
    return this.active.length ? this.active.at(-1).scope() : this.call.scope;
  }

  leaveLoops(line) {
    while (this.active.length && (line === null || !this.active.at(-1).contains(line))) {
      const loop = this.active.pop();
      const iterations = loop.run.iterations;
      const last = iterations.at(-1);
      if (last.events.length === 1 && !last.loops.length) {
        iterations.pop();
        this.scope().events.push(last.events[0]);
      }
      if (!iterations.length) {
        const loops = this.scope().loops;
        loops.splice(loops.indexOf(loop.run), 1);
      }
    }
  }

  enterLine(line) {
    this.leaveLoops(line);
    const info = this.loops[line];
    if (info !== undefined) {
      if (!(this.active.length && this.active.at(-1).info === info)) {
        const run = { line, iterations: [] };
        this.scope().loops.push(run);
        this.active.push(new ActiveLoop(info, run));
      }
      this.active.at(-1).run.iterations.push(newScope());
      this.labelled = info.targets !== null ? this.active.at(-1) : null;
    } else {
      this.labelled = null;
    }
    this.pending = { line, values: [] };
    this.scope().events.push(this.pending);
  }

  finishPending(values, nextLine) {
    const pending = this.pending;
    if (pending !== null) {
      const body = this.conditions[pending.line];
      if (body !== undefined && nextLine !== null) {
        pending.condition = body[0] <= nextLine && nextLine <= body[1];
      }
      if (values !== null) {
        const assigned = this.assigned[pending.line] ?? [];
        pending.values = Object.entries(values)
          .filter(([name, text]) => assigned.includes(name) || this.values[name] !== text)
          .map(([name, text]) => [name, shorten(text)]);
      }
    }
    if (this.labelled !== null && values !== null) {
      this.labelled.scope().label = this.labelled.info.targets
        .filter((name) => name in values)
        .map((name) => `${name}=${shorten(values[name])}`)
        .join(", ");
    }
    this.labelled = null;
    if (values !== null) {
      Object.assign(this.values, values);
    }
  }
}

function run(input) {
  const deadline = performance.now() + Number(timeoutMs);
  const limit = Number(maxSteps);
  const calls = [];
  const recorders = [];
  const exceptions = new Set();
  let steps = 0;

  function countStep() {
    steps += 1;
    if (steps > limit) {
      throw new Stopped("steps");
    }
    if (performance.now() > deadline) {
      throw new Stopped("timeout");
    }
  }

  globalThis.__xray = {
    enter(snap) {
      const call = { depth: recorders.length, scope: newScope() };
      calls.push(call);
      recorders.push(new FrameRecorder(input.analysis, call, snapshot(snap)));
    },
    line(line, snap) {
      const recorder = recorders.at(-1);
      if (recorder.pending !== null && recorder.pending.line === line) {
        return;
      }
      countStep();
      recorder.finishPending(snapshot(snap), line);
      recorder.enterLine(line);
    },
    ret(value, snap) {
      const recorder = recorders.at(-1);
      recorder.finishPending(snapshot(snap), null);
      recorder.leaveLoops(null);
      recorder.finished = true;
      if (recorder.pending !== null) {
        recorder.pending.return = shorten(fullRepr(value));
      }
      return value;
    },
    raise(error) {
      const recorder = recorders.at(-1);
      if (error instanceof Stopped || recorder.pending === null || exceptions.has(error)) {
        return;
      }
      exceptions.add(error);
      recorder.pending.exception = errorMessage(error);
    },
    exit() {
      const recorder = recorders.pop();
      if (!recorder.finished) {
        recorder.finishPending(null, null);
        recorder.leaveLoops(null);
      }
    },
  };

  function traced(stopped) {
    return {
      status: "traced",
      function: input.function,
      loops: Object.values(input.analysis.loops)
        .sort((a, b) => a.line - b.line)
        .map((info) => ({ line: info.line, end_line: info.end_line })),
      calls,
      stopped,
    };
  }

  const fileUrl = pathToFileURL(file).href;
  registerHooks({
    resolve(specifier, context, nextResolve) {
      if (context.parentURL?.startsWith("data:")) {
        return nextResolve(specifier, { ...context, parentURL: fileUrl });
      }
      return nextResolve(specifier, context);
    },
  });
  globalThis.require = createRequire(file);
  globalThis.__filename = file;
  globalThis.__dirname = path.dirname(file);

  return (async () => {
    try {
      const code = input.typescript ? stripTypeScriptTypes(input.code) : input.code;
      await import("data:text/javascript," + encodeURIComponent(code));
      return traced(null);
    } catch (error) {
      if (error instanceof Stopped) {
        return traced(error.reason);
      }
      if (!calls.length) {
        return { status: "failed", message: errorMessage(error) };
      }
      return traced(null);
    }
  })();
}

run(JSON.parse(fs.readFileSync(0, "utf8"))).then((result) => {
  fs.writeFileSync(output, JSON.stringify(result));
});
