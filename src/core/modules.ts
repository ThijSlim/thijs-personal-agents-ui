import type { FC } from "react";
import { agentsModule } from "../modules/agents";

export type Selection = { module: string; id: string } | null;

export interface AppModule {
  id: string;
  title: string;
  Sidebar?: FC<{ selection: Selection; onSelect(s: Selection): void }>;
  Main?: FC<{ selection: Selection }>;
}

export const modules: AppModule[] = [agentsModule];
