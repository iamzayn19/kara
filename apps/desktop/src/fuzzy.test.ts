import { describe, expect, it } from "vitest";
import { fuzzyFilter, fuzzyMatch, fuzzyScore } from "./fuzzy";

const files = [
  "apps/desktop/src/main.ts",
  "apps/desktop/src/terminal.ts",
  "crates/kara-cli/src/main.rs",
  "crates/kara-cli/src/serve.rs",
  "crates/kara-domain/src/maintenance_test.ts",
  "README.md",
];

const rank = (q: string) => fuzzyFilter(q, files, (f) => f).map((r) => r.item);

describe("fuzzyMatch", () => {
  it("requires every query character in order", () => {
    expect(fuzzyMatch("mts", "main.ts")).not.toBeNull();
    expect(fuzzyMatch("stm", "main.ts")).toBeNull();
    expect(fuzzyMatch("xyz", "main.ts")).toBeNull();
  });

  it("is case-insensitive and ignores spaces in the query", () => {
    expect(fuzzyMatch("MAIN", "main.ts")).not.toBeNull();
    expect(fuzzyMatch("ma in", "main.ts")).not.toBeNull();
  });

  it("returns positions that spell the query", () => {
    const c = "apps/desktop/src/main.ts";
    const m = fuzzyMatch("mts", c)!;
    expect(m.positions.map((i) => c[i].toLowerCase()).join("")).toBe("mts");
  });

  it("an empty query matches everything with no highlights", () => {
    expect(fuzzyMatch("", "anything")).toEqual({ score: 0, positions: [] });
  });
});

describe("fuzzyFilter ranking", () => {
  it("prefers a file-name prefix over a deep directory match", () => {
    expect(rank("main")[0]).toMatch(/\/main\.(ts|rs)$/);
    expect(rank("main").indexOf("crates/kara-domain/src/maintenance_test.ts")).toBeGreaterThan(1);
  });

  it("uses the directory to disambiguate", () => {
    expect(rank("cli main")[0]).toBe("crates/kara-cli/src/main.rs");
    expect(rank("desk main")[0]).toBe("apps/desktop/src/main.ts");
  });

  it("finds abbreviations across word boundaries", () => {
    expect(rank("term")[0]).toBe("apps/desktop/src/terminal.ts");
    expect(rank("srv")[0]).toBe("crates/kara-cli/src/serve.rs");
  });

  it("drops non-matches and respects the limit", () => {
    expect(rank("qqq")).toEqual([]);
    expect(fuzzyFilter("", files, (f) => f, 2)).toHaveLength(2);
  });
});

describe("fuzzy quality and speed", () => {
  it("highlights the contiguous run in the file name, not scattered letters", () => {
    const c = "m/a/i/n/main.ts";
    expect(fuzzyMatch("main", c)!.positions).toEqual([8, 9, 10, 11]);
  });

  it("filters 20,000 paths fast enough to run on every keystroke", () => {
    const many = Array.from({ length: 20_000 }, (_, i) => `pkg${i % 97}/src/module_${i}/component${i}.tsx`);
    many.push("crates/kara-cli/src/serve.rs");
    const t0 = performance.now();
    const r = fuzzyFilter("kclisrv", many, (f) => f);
    const ms = performance.now() - t0;
    expect(r[0].item).toBe("crates/kara-cli/src/serve.rs");
    expect(ms).toBeLessThan(250);
  });
});

describe("fuzzyScore", () => {
  it("agrees with fuzzyMatch on every candidate", () => {
    const cands = [...files, "m/a/i/n/main.ts", "fooBarBaz.ts", "a", ""];
    for (const q of ["main", "mts", "cli main", "fbb", "srv", "zzz", "a"]) {
      for (const c of cands) {
        const full = fuzzyMatch(q, c);
        const fast = fuzzyScore(q, c);
        if (full === null) expect(fast).toBeNull();
        else expect(fast).toBeCloseTo(full.score, 9);
      }
    }
  });
});

