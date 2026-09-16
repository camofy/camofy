import { createContext, useContext } from "react";
import type { Resource, Token, Issued, User } from "./model";
export type Workspace = {
  resources: Resource[];
  tokens: Token[];
  user: User;
  loading: boolean;
  busy: boolean;
  error: string;
  notice: string;
  connected: boolean;
  load: () => Promise<void>;
  run: <T>(fn: () => Promise<T>, message?: string) => Promise<T | undefined>;
  save: (r: Resource) => Promise<Resource | undefined>;
  issue: (bundle: string, device?: string) => Promise<Issued | undefined>;
};
export const WorkspaceContext = createContext<Workspace | null>(null);
export function useWorkspace() {
  const ctx = useContext(WorkspaceContext);
  if (!ctx) throw new Error("Workspace unavailable");
  return ctx;
}
