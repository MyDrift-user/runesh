/**
 * Background service worker.
 *
 * - Runs in a separate context (no DOM access)
 * - Not persistent (Chrome terminates after ~30s of inactivity)
 * - Use chrome.storage for state, not global variables
 * - Keep lightweight (no React or heavy frameworks)
 */

// Example: typed message handler
// import { createMessageHandler } from "@runesh/ui/lib/chrome-messaging";
//
// interface Messages {
//   FETCH_DATA: { args: { url: string }; return: { data: unknown } };
// }
//
// const handler = createMessageHandler<Messages>();
// handler.on("FETCH_DATA", async ({ url }) => {
//   const res = await fetch(url);
//   return { data: await res.json() };
// });
// handler.listen();

export default defineBackground(() => {
  console.log("Background service worker started");
});
