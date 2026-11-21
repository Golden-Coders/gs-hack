import { describe, expect, it } from "vitest";
import { isTypingTarget, type KeyTarget } from "./keyboard";

const plain = (extra: Partial<KeyTarget> = {}): KeyTarget => ({ tagName: "DIV", closest: () => null, ...extra });

describe("isTypingTarget", () => {
  it.each(["INPUT", "TEXTAREA", "SELECT", "input", "textarea", "select", "Input", "TextArea"])("pola formularza (%s)", (tagName) => {
    expect(isTypingTarget({ tagName })).toBe(true);
  });

  it("isContentEditable", () => {
    expect(isTypingTarget(plain({ isContentEditable: true }))).toBe(true);
  });

  it("closest zwraca element dla combobox/dialog/contenteditable", () => {
    for (const hit of ['[role="combobox"]', '[role="dialog"]', '[contenteditable="true"]']) {
      const seen: string[] = [];
      const el = plain({
        closest: (sel) => {
          seen.push(sel);
          return sel.includes(hit) ? { tagName: "DIV" } : null;
        },
      });
      expect(isTypingTarget(el)).toBe(true);
      expect(seen[0]).toContain(hit);
    }
  });

  it("zwykły element i brak closest nie są polem tekstowym", () => {
    expect(isTypingTarget(plain())).toBe(false);
    expect(isTypingTarget({ tagName: "DIV" })).toBe(false);
    expect(isTypingTarget({})).toBe(false);
  });

  it("null i undefined", () => {
    expect(isTypingTarget(null)).toBe(false);
    expect(isTypingTarget(undefined)).toBe(false);
  });
});
