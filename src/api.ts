import { invoke } from "@tauri-apps/api/core";

export type Account = { id: string; label: string; config_dir: string };
export type Project = { id: string; label: string; path: string; account: string };
export type Config = { terminal: string; accounts: Account[]; projects: Project[] };
export type TerminalInfo = { id: string; label: string };

export type InheritDecision = "merge" | "skip";
export type InheritStatus = "inherited" | "skipped" | "conflict" | "none";
export type InheritSubdirStatus = {
  subdir: string;
  status: InheritStatus;
  decision: InheritDecision | null;
};

export const getConfig = () => invoke<Config>("get_config");
export const saveConfig = (config: Config) => invoke<void>("save_config", { config });
export const listTerminals = () => invoke<TerminalInfo[]>("list_terminals");

export const getInheritStatus = (accountId: string) =>
  invoke<InheritSubdirStatus[]>("get_inherit_status", { accountId });
export const setInheritDecision = (
  accountId: string,
  subdir: string,
  decision: InheritDecision,
) => invoke<void>("set_inherit_decision", { accountId, subdir, decision });
