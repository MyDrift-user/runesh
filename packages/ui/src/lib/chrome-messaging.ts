/// <reference types="chrome" />
/**
 * Type-safe Chrome extension messaging utilities.
 *
 * Define your message types once, get type checking on send and receive:
 *
 * ```ts
 * interface Messages {
 *   FETCH_DATA: { args: { url: string }; return: { data: unknown } };
 *   GET_STATUS: { args: {}; return: { active: boolean } };
 * }
 *
 * // In popup/content script:
 * const send = createMessageSender<Messages>();
 * const result = await send("FETCH_DATA", { url: "https://..." });
 *
 * // In background service worker:
 * const handler = createMessageHandler<Messages>();
 * handler.on("FETCH_DATA", async ({ url }) => {
 *   const res = await fetch(url);
 *   return { data: await res.json() };
 * });
 * handler.listen();
 * ```
 */

type MessageArgs = Record<string, unknown>;

interface MessageMap {
  [type: string]: { args: MessageArgs; return: unknown };
}

// ── Sender (popup, content script, options page) ────────────────────────────

type SendFn<T extends MessageMap> = <K extends keyof T & string>(
  type: K,
  args: T[K]["args"],
) => Promise<T[K]["return"]>;

export function createMessageSender<T extends MessageMap>(): SendFn<T> {
  return (type, args) => {
    return new Promise((resolve, reject) => {
      if (typeof chrome === "undefined" || !chrome.runtime) {
        reject(new Error("Chrome runtime not available"));
        return;
      }
      chrome.runtime.sendMessage({ type, ...args }, (response) => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message));
        } else {
          resolve(response);
        }
      });
    });
  };
}

/**
 * Send a message to a specific tab's content script.
 */
export function createTabMessageSender<T extends MessageMap>(): <K extends keyof T & string>(
  tabId: number,
  type: K,
  args: T[K]["args"],
) => Promise<T[K]["return"]> {
  return (tabId, type, args) => {
    return new Promise((resolve, reject) => {
      chrome.tabs.sendMessage(tabId, { type, ...args }, (response) => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message));
        } else {
          resolve(response);
        }
      });
    });
  };
}

// ── Handler (background service worker) ─────────────────────────────────────

type Handler<A, R> = (args: A) => Promise<R> | R;

interface MessageHandler<T extends MessageMap> {
  on: <K extends keyof T & string>(
    type: K,
    handler: Handler<T[K]["args"], T[K]["return"]>,
  ) => void;
  listen: () => void;
}

export function createMessageHandler<T extends MessageMap>(): MessageHandler<T> {
  const handlers = new Map<string, Handler<unknown, unknown>>();

  return {
    on(type, handler) {
      handlers.set(type, handler as Handler<unknown, unknown>);
    },
    listen() {
      chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
        const { type, ...args } = message;
        const handler = handlers.get(type);
        if (!handler) return false;

        // Handle async handlers
        Promise.resolve(handler(args))
          .then(sendResponse)
          .catch((err) => sendResponse({ error: err.message }));

        return true; // Keep message channel open for async response
      });
    },
  };
}
