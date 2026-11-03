import { mockConvertFileSrc, mockIPC } from "@tauri-apps/api/mocks";
import { expect, test, vi } from "vitest";
import { type CoreEvent, commands, messageHeader, protocolUrl, subscribe } from ".";

test("a command's JerryError comes back typed", async () => {
  mockIPC(() => Promise.reject({ code: "NotFound", path: String.raw`C:\gone.txt` }));

  const result = await commands.devSampleError();

  expect(result).toEqual({
    status: "error",
    error: { code: "NotFound", path: String.raw`C:\gone.txt` },
  });
});

test("a rejection that isn't a JerryError is thrown", async () => {
  mockIPC(() => Promise.reject("command dev_sample_error not found"));

  await expect(commands.devSampleError()).rejects.toThrow("command dev_sample_error not found");
});

test("messageHeader reads the little-endian header without copying", () => {
  const message = new ArrayBuffer(24);
  const words = new DataView(message);
  words.setUint32(0, 2, true);
  words.setUint32(4, 1000, true);
  words.setUint32(8, 7, true);

  const header = messageHeader(message);

  expect([...header]).toEqual([2, 1000, 7, 0]);
  expect(header.buffer).toBe(message);
  expect(() => messageHeader(new ArrayBuffer(8))).toThrow(RangeError);
});

function fakeEvent<T>() {
  const handlers: ((event: { payload: T }) => void)[] = [];
  const stop = vi.fn();
  let register: () => void = () => {};
  const registered = new Promise<void>((resolve) => {
    register = resolve;
  });
  const event: CoreEvent<T> = {
    listen: async (handler) => {
      handlers.push(handler);
      await registered;
      return stop;
    },
  };
  const emit = (payload: T) => {
    for (const handler of handlers) handler({ payload });
  };
  return { event, emit, stop, register };
}

/** Lets pending promise callbacks run. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

test("subscribe delivers payloads until it is stopped", async () => {
  const { event, emit, stop, register } = fakeEvent<string>();
  const seen: string[] = [];

  const unsubscribe = subscribe(event, (payload) => seen.push(payload));
  register();
  await settle();
  emit("first");
  unsubscribe();
  emit("second");

  expect(seen).toEqual(["first"]);
  expect(stop).toHaveBeenCalledOnce();
});

test("subscribe reports when it is listening", async () => {
  const { event, register } = fakeEvent<string>();
  const listening = vi.fn();

  const unsubscribe = subscribe(event, () => {}, listening);
  await settle();
  expect(listening).not.toHaveBeenCalled();
  register();
  await settle();
  unsubscribe();

  expect(listening).toHaveBeenCalledOnce();
});

test("subscribe stops a listener that registers after it was stopped", async () => {
  const { event, emit, stop, register } = fakeEvent<string>();
  const seen: string[] = [];

  const unsubscribe = subscribe(event, (payload) => seen.push(payload));
  unsubscribe();
  emit("before registration");
  register();
  await settle();
  emit("after registration");

  expect(stop).toHaveBeenCalledOnce();
  expect(seen).toEqual([]);
});

test("protocolUrl points at Jerry's protocol host", () => {
  mockConvertFileSrc("windows");

  expect(protocolUrl("icon", "ext.dwg-32")).toBe("http://icon.localhost/ext.dwg-32");
});
