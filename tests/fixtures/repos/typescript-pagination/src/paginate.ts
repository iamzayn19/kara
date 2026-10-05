export interface Page<T> {
  items: T[];
  page: number;
  perPage: number;
  totalPages: number;
}

/** Return one page of `items`. Pages are 1-based. */
export function paginate<T>(items: T[], page: number, perPage: number): Page<T> {
  const totalPages = Math.max(1, Math.ceil(items.length / perPage));
  const start = page * perPage;
  return {
    items: items.slice(start, start + perPage),
    page,
    perPage,
    totalPages,
  };
}
