import { invoke } from "@tauri-apps/api/core";

export type Account = { id: string; label: string; config_dir: string };
export type Project = { id: string; label: string; path: string; default_account?: string | null };
export type Config = { terminal: string; accounts: Account[]; projects: Project[] };
export type TerminalInfo = { id: string; label: string };

export const getConfig = () => invoke<Config>("get_config");
export const saveConfig = (config: Config) => invoke<void>("save_config", { config });
export const listTerminals = () => invoke<TerminalInfo[]>("list_terminals");
