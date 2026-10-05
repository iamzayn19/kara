import { paginate, type Page } from "./paginate.ts";

export interface User {
  id: number;
  name: string;
}

export function listUsers(users: User[], params: { page?: string; perPage?: string }): Page<User> {
  const page = Number(params.page ?? "1");
  const perPage = Number(params.perPage ?? "10");
  return paginate(users, page, perPage);
}
