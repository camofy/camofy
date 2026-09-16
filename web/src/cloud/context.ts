import { createContext, useContext } from "react";
import type { Resource, User } from "./model";
export type Workspace = {
  resources: Resource[];
  user: User;
  loading: boolean;
  busy: boolean;
  error: string;
  notice: string;
  connected: boolean;
  load: () => Promise<void>;
  run: <T>(fn: () => Promise<T>, message?: string) => Promise<T | undefined>;
  save: (r: Resource) => Promise<Resource | undefined>;
};
export const WorkspaceContext = createContext<Workspace | null>(null);
export function useWorkspace() {
  const ctx = useContext(WorkspaceContext);
  if (!ctx) throw new Error("Workspace unavailable");
  return ctx;
}
