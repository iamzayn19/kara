// Fuzzy matching for quick open and the command palette: the query's
// characters must appear in order (case-insensitive); the score rewards
// matches that start words or path segments, run consecutively, and land
// in the file name rather than its directory.

export interface FuzzyMatch {
  score: number;
  /** Indices into the candidate string, for highlighting. */
  positions: number[];
}

const SEPARATORS = "/\\_-. ";
const BONUS_BOUNDARY = 8;
const BONUS_CONSECUTIVE = 5;
const BONUS_IN_NAME = 3;
const BONUS_NAME_PREFIX = 20;

function isBoundary(s: string, i: number): boolean {
  if (i === 0) return true;
  const prev = s[i - 1];
  if (SEPARATORS.includes(prev)) return true;
  // camelCase hump: "fooBar" -> B
  return prev === prev.toLowerCase() && s[i] !== s[i].toLowerCase();
}

export function fuzzyMatch(query: string, candidate: string): FuzzyMatch | null {
  const q = query.toLowerCase().replace(/\s+/g, "");
  if (!q) return { score: 0, positions: [] };
  const c = candidate.toLowerCase();
  const n = c.length;
  const m = q.length;

  // Cheap reject: most candidates don't contain the query in order at all.
  for (let i = 0, k = 0; k < m; i++) {
    if (i >= n) return null;
    if (c[i] === q[k]) k++;
  }

  const nameStart = Math.max(candidate.lastIndexOf("/"), candidate.lastIndexOf("\\")) + 1;
  const base = new Float64Array(n);
  for (let i = 0; i < n; i++) {
    base[i] = 1 + (isBoundary(candidate, i) ? BONUS_BOUNDARY : 0) + (i >= nameStart ? BONUS_IN_NAME : 0);
  }

  // best[k][i]: best score with q[k] placed at i. from[k][i]: where q[k-1] was.
  const NEG = -Infinity;
  const best: Float64Array[] = [];
  const from: Int32Array[] = [];
  for (let k = 0; k < m; k++) {
    const row = new Float64Array(n).fill(NEG);
    const back = new Int32Array(n).fill(-1);
    let prefixMax = NEG; // best of best[k-1][p] for p < i - 1
    let prefixArg = -1;
    for (let i = 0; i < n; i++) {
      if (k > 0 && i >= 2 && best[k - 1][i - 2] > prefixMax) {
        prefixMax = best[k - 1][i - 2];
        prefixArg = i - 2;
      }
      if (c[i] !== q[k]) continue;
      if (k === 0) {
        row[i] = base[i];
        continue;
      }
      const consec = i >= 1 ? best[k - 1][i - 1] + BONUS_CONSECUTIVE : NEG;
      if (consec >= prefixMax && consec > NEG) {
        row[i] = consec + base[i];
        back[i] = i - 1;
      } else if (prefixMax > NEG) {
        row[i] = prefixMax + base[i];
        back[i] = prefixArg;
      }
    }
    best.push(row);
    from.push(back);
  }

  let end = -1;
  let score = NEG;
  for (let i = 0; i < n; i++) {
    if (best[m - 1][i] > score) {
      score = best[m - 1][i];
      end = i;
    }
  }
  if (end === -1) return null;

  const positions = new Array<number>(m);
  for (let k = m - 1, i = end; k >= 0; k--) {
    positions[k] = i;
    i = from[k][i];
  }

  if (c.startsWith(q, nameStart)) score += BONUS_NAME_PREFIX;
  // Shorter candidates win ties: "main.ts" over "domain/main.test.ts".
  score -= n * 0.05;
  return { score, positions };
}

// Rolling rows reused across calls by fuzzyScore, grown on demand.
let rowPrev = new Float64Array(256);
let rowCur = new Float64Array(256);

/** The same score as fuzzyMatch, without positions: two reused rows instead
 * of a full table, so ranking thousands of paths per keystroke stays cheap. */
export function fuzzyScore(query: string, candidate: string): number | null {
  const q = query.toLowerCase().replace(/\s+/g, "");
  if (!q) return 0;
  const c = candidate.toLowerCase();
  const n = c.length;
  const m = q.length;
  for (let i = 0, k = 0; k < m; i++) {
    if (i >= n) return null;
    if (c[i] === q[k]) k++;
  }
  if (rowPrev.length < n) {
    rowPrev = new Float64Array(n * 2);
    rowCur = new Float64Array(n * 2);
  }
  const nameStart = Math.max(candidate.lastIndexOf("/"), candidate.lastIndexOf("\\")) + 1;
  const NEG = -Infinity;
  let prev = rowPrev;
  let cur = rowCur;
  for (let k = 0; k < m; k++) {
    let prefixMax = NEG;
    const qk = q[k];
    for (let i = 0; i < n; i++) {
      if (k > 0 && i >= 2 && prev[i - 2] > prefixMax) prefixMax = prev[i - 2];
      if (c[i] !== qk) {
        cur[i] = NEG;
        continue;
      }
      const base = 1 + (isBoundary(candidate, i) ? BONUS_BOUNDARY : 0) + (i >= nameStart ? BONUS_IN_NAME : 0);
      if (k === 0) {
        cur[i] = base;
        continue;
      }
      const consec = i >= 1 ? prev[i - 1] + BONUS_CONSECUTIVE : NEG;
      const from = consec >= prefixMax ? consec : prefixMax;
      cur[i] = from > NEG ? from + base : NEG;
    }
    const t = prev;
    prev = cur;
    cur = t;
  }
  let score = NEG;
  for (let i = 0; i < n; i++) if (prev[i] > score) score = prev[i];
  if (score === NEG) return null;
  if (c.startsWith(q, nameStart)) score += BONUS_NAME_PREFIX;
  return score - n * 0.05;
}

/** Ranks candidates by fuzzy score, best first, dropping non-matches.
 * Highlight positions are only computed for the results returned. */
export function fuzzyFilter<T>(query: string, items: T[], key: (t: T) => string, limit = 50): { item: T; match: FuzzyMatch }[] {
  const scored: { item: T; score: number }[] = [];
  for (const item of items) {
    const score = fuzzyScore(query, key(item));
    if (score !== null) scored.push({ item, score });
  }
  scored.sort((a, b) => b.score - a.score);
  return scored.slice(0, limit).map(({ item }) => ({ item, match: fuzzyMatch(query, key(item))! }));
}
