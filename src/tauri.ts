import { invoke as tauriInvoke } from "@tauri-apps/api/core";

export const isTauri = () => "__TAURI_INTERNALS__" in window;

export async function invokeCommand<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    throw new Error("当前页面未运行在 Tauri 环境中");
  }

  return tauriInvoke<T>(command, args);
}
