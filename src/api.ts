import { invoke } from "@tauri-apps/api/core";

export type Account = { id: string; label: string; config_dir: string };
export type Project = { id: string; label: string; path: string; account: string };
// Calibrated token ceilings for the tray usage lines. `null` = no ceiling set
// (the tray then shows raw tokens instead of a percentage).
export type UsageLimits = { session_tokens: number | null; weekly_tokens: number | null };
export type Config = {
  terminal: string;
  accounts: Account[];
  projects: Project[];
  usage_limits: UsageLimits;
};
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
