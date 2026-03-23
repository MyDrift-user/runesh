import { defineConfig } from "wxt";

export default defineConfig({
  modules: ["@wxt-dev/module-react"],
  manifest: {
    name: "YOUR_APP",
    description: "YOUR_APP Chrome Extension",
    permissions: ["storage"],
    // Add host_permissions for content scripts:
    // host_permissions: ["*://*.example.com/*"],
  },
});
