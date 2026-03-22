/**
 * Type-safe wrapper around Tauri's invoke API.
 *
 * Usage:
 * ```ts
 * // Define your commands
 * interface Commands {
 *   get_config: { args: {}; return: AppConfig };
 *   save_config: { args: { server: string; key: string }; return: string };
 *   get_status: { args: {}; return: StatusInfo };
 * }
 *
 * const invoke = createInvoke<Commands>();
 * const config = await invoke("get_config", {});
 * await invoke("save_config", { server: "https://...", key: "abc" });
 * ```
 */

type InvokeArgs = Record<string, unknown>;

interface CommandMap {
  [command: string]: { args: InvokeArgs; return: unknown };
}

type InvokeFn<T extends CommandMap> = <K extends keyof T & string>(
  command: K,
  args: T[K]["args"],
) => Promise<T[K]["return"]>;

export function createInvoke<T extends CommandMap>(): InvokeFn<T> {
  return async (command, args) => {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke(command, args) as Promise<T[typeof command]["return"]>;
  };
}

/**
 * Listen to events from the Tauri backend.
 *
 * Usage:
 * ```ts
 * const unlisten = await tauriListen<StatusUpdate>("status-changed", (event) => {
 *   console.log(event.payload);
 * });
 * // Later: unlisten();
 * ```
 */
export async function tauriListen<T>(
  event: string,
  handler: (event: { payload: T }) => void,
): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<T>(event, handler);
}
