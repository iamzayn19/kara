/** The key a shortcut refers to, from its physical position (`e.code`)
 * rather than the character it typed (`e.key`): with an Urdu or Arabic
 * layout Cmd+S types "س", and on macOS Option turns S into "ß", but
 * shortcuts must still work. Falls back to `e.key` for other keys. */
export function shortcutKey(e: { code: string; key: string }): string {
  if (/^Key[A-Z]$/.test(e.code)) return e.code.slice(3).toLowerCase();
  if (/^Digit[0-9]$/.test(e.code)) return e.code.slice(5);
  const named: Record<string, string> = { Comma: ",", Period: ".", Slash: "/", Backquote: "`" };
  return named[e.code] ?? e.key.toLowerCase();
}
