/** Absolute path of a workspace-relative, "/"-separated path, using the
 * root's own separator (backslashes for a Windows root). */
export function joinWorkspacePath(root: string, rel: string): string {
  const sep = root.includes("\\") ? "\\" : "/";
  const base = root.length > 1 ? root.replace(/[\\/]+$/, "") : root;
  if (!rel) return base;
  return (base.endsWith(sep) ? base : base + sep) + rel.split("/").join(sep);
}
