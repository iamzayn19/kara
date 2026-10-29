import { describe, expect, it } from "vitest";
import { shortcutKey } from "./keys";

describe("shortcutKey", () => {
  it("uses the physical key, not the typed character", () => {
    expect(shortcutKey({ code: "KeyS", key: "s" })).toBe("s");
    expect(shortcutKey({ code: "KeyS", key: "S" })).toBe("s");
    // macOS Option+S, and Cmd+S with an Urdu layout.
    expect(shortcutKey({ code: "KeyS", key: "ß" })).toBe("s");
    expect(shortcutKey({ code: "KeyS", key: "س" })).toBe("s");
    expect(shortcutKey({ code: "Comma", key: "،" })).toBe(",");
    expect(shortcutKey({ code: "Digit1", key: "!" })).toBe("1");
  });

  it("falls back to the key name for everything else", () => {
    expect(shortcutKey({ code: "Tab", key: "Tab" })).toBe("tab");
    expect(shortcutKey({ code: "Escape", key: "Escape" })).toBe("escape");
  });
});
