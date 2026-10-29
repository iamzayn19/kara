import { describe, expect, it } from "vitest";
import { joinWorkspacePath } from "./paths";

describe("joinWorkspacePath", () => {
  it("joins Unix paths", () => {
    expect(joinWorkspacePath("/home/me/proj", "src/a.ts")).toBe("/home/me/proj/src/a.ts");
    expect(joinWorkspacePath("/home/me/proj/", "src/a.ts")).toBe("/home/me/proj/src/a.ts");
  });

  it("uses backslashes under a Windows root", () => {
    expect(joinWorkspacePath("C:\\Users\\me\\proj", "src/lib/a.ts")).toBe("C:\\Users\\me\\proj\\src\\lib\\a.ts");
    expect(joinWorkspacePath("C:\\Users\\me\\proj\\", "a.ts")).toBe("C:\\Users\\me\\proj\\a.ts");
  });

  it("returns the root itself for an empty path", () => {
    expect(joinWorkspacePath("/home/me/proj/", "")).toBe("/home/me/proj");
    expect(joinWorkspacePath("/", "")).toBe("/");
    expect(joinWorkspacePath("/", "a.ts")).toBe("/a.ts");
  });
});
