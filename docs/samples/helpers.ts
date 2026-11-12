import { LIMIT } from "./helper_module.ts";
import { isDeepStrictEqual } from "node:util";

function double(x: number): number {
  return x * 2;
}

function scaled(values: number[]): number {
  const result = values.filter((v) => v < LIMIT).map(double);
  const same = isDeepStrictEqual(result, [2, 14]);
  return Math.sqrt(result.reduce((a, b) => a + b, 0));
}

console.log("side effect");
scaled([1, 7, 30]);
