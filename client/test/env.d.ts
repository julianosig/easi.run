// The few Node.js test APIs the unit tests use (no @types/node dependency), and text imports (bundled by esbuild).

declare module 'node:test' {
  interface TestContext {
    mock: { timers: { enable(options: { apis: 'setTimeout'[] }): void; tick(ms: number): void } };
  }
  export function test(name: string, fn: (t: TestContext) => void | Promise<void>): void;
}

declare module 'node:assert/strict' {
  interface Assert {
    (value: unknown, message?: string): asserts value;
    equal(actual: unknown, expected: unknown, message?: string): void;
    deepEqual(actual: unknown, expected: unknown, message?: string): void;
    throws(fn: () => unknown, expected?: ((e: unknown) => boolean) | RegExp | object, message?: string): void;
  }
  const assert: Assert;
  export default assert;
}

declare module '*.txt' {
  const text: string;
  export default text;
}
