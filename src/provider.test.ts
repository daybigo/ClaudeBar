import { describe, it, expect } from "vitest";
import { isProvider, resolveProvider, PROVIDERS } from "./provider";

describe("isProvider", () => {
  it("acepta los proveedores válidos", () => {
    expect(isProvider("claude")).toBe(true);
    expect(isProvider("codex")).toBe(true);
    expect(isProvider("antigravity")).toBe(true);
  });
  it("rechaza valores inválidos", () => {
    expect(isProvider("gemini")).toBe(false);
    expect(isProvider("")).toBe(false);
    expect(isProvider(null)).toBe(false);
    expect(isProvider(42)).toBe(false);
  });
});

describe("resolveProvider", () => {
  it("devuelve el proveedor guardado si es válido", () => {
    expect(resolveProvider("codex")).toBe("codex");
    expect(resolveProvider("antigravity")).toBe("antigravity");
  });
  it("cae a 'claude' por defecto cuando falta o es inválido", () => {
    expect(resolveProvider(null)).toBe("claude");
    expect(resolveProvider("")).toBe("claude");
    expect(resolveProvider("foo")).toBe("claude");
  });
});

describe("PROVIDERS", () => {
  it("lista los tres proveedores con claude primero", () => {
    expect(PROVIDERS).toEqual(["claude", "codex", "antigravity"]);
  });
});
